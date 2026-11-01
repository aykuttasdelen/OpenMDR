use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OCRProgressPayload {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub page: usize,
    #[serde(rename = "totalPages")]
    pub total_pages: usize,
    pub percent: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OCRPage {
    #[serde(rename = "pageNumber")]
    pub page_number: usize,
    #[serde(rename = "isOcr")]
    pub is_ocr: bool,
    pub text: String,
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OCRResult {
    pub success: bool,
    #[serde(rename = "totalPages")]
    pub total_pages: usize,
    #[serde(rename = "processedPages")]
    pub processed_pages: usize,
    pub markdown: String,
    pub pages: Vec<OCRPage>,
    #[serde(default, rename = "extractedImages")]
    pub extracted_images: Vec<String>,
    pub error: Option<String>,
}

enum OCRExecutionTarget {
    CompiledBinary(PathBuf),
    SwiftScript(PathBuf),
}

fn is_newer(a: &Path, b: &Path) -> bool {
    match (
        std::fs::metadata(a).and_then(|m| m.modified()),
        std::fs::metadata(b).and_then(|m| m.modified()),
    ) {
        (Ok(ta), Ok(tb)) => ta > tb,
        _ => false,
    }
}

fn resolve_ocr_target(app: &AppHandle) -> Result<OCRExecutionTarget, String> {
    let mut found_bin: Option<PathBuf> = None;
    let mut found_script: Option<PathBuf> = None;

    // Prefer workspace script in development if present
    let script_candidates = [
        "native/ocr_engine.swift",
        "src-tauri/native/ocr_engine.swift",
        "../src-tauri/native/ocr_engine.swift",
    ];
    for c in &script_candidates {
        let path = Path::new(c);
        if path.exists() {
            found_script = Some(path.to_path_buf());
            break;
        }
    }

    // 1. Check bundle resources directory
    if let Ok(res_dir) = app.path().resource_dir() {
        for rel in ["native/ocr_engine", "ocr_engine"] {
            let bin = res_dir.join(rel);
            if bin.exists() && found_bin.is_none() {
                found_bin = Some(bin);
            }
        }
        if found_script.is_none() {
            for rel in ["native/ocr_engine.swift", "ocr_engine.swift"] {
                let script = res_dir.join(rel);
                if script.exists() {
                    found_script = Some(script);
                    break;
                }
            }
        }
    }

    // 2. Check next to current executable
    if found_bin.is_none() {
        if let Ok(curr_exe) = std::env::current_exe() {
            if let Some(parent) = curr_exe.parent() {
                for rel in ["ocr_engine", "native/ocr_engine"] {
                    let candidate = parent.join(rel);
                    if candidate.exists() {
                        found_bin = Some(candidate);
                        break;
                    }
                }
            }
        }
    }

    // 3. Check workspace / relative directories for binary
    if found_bin.is_none() {
        let bin_candidates = [
            "native/ocr_engine",
            "src-tauri/native/ocr_engine",
            "../src-tauri/native/ocr_engine",
        ];
        for c in &bin_candidates {
            let path = Path::new(c);
            if path.exists() {
                found_bin = Some(path.to_path_buf());
                break;
            }
        }
    }

    // If script is available, compile/cache to temp_dir outside src-tauri watcher whenever script is newer than cached_bin
    if let Some(ref script) = found_script {
        let cached_bin = std::env::temp_dir().join("openmd_ocr_engine");
        if !cached_bin.exists() || is_newer(script, &cached_bin) {
            let status = std::process::Command::new("swiftc")
                .arg("-O")
                .arg(script)
                .arg("-o")
                .arg(&cached_bin)
                .status();
            if let Ok(s) = status
                && s.success()
                && cached_bin.exists()
            {
                return Ok(OCRExecutionTarget::CompiledBinary(cached_bin));
            }
        } else {
            return Ok(OCRExecutionTarget::CompiledBinary(cached_bin));
        }
    }

    if let Some(bin) = found_bin {
        return Ok(OCRExecutionTarget::CompiledBinary(bin));
    }
    if let Some(script) = found_script {
        return Ok(OCRExecutionTarget::SwiftScript(script));
    }

    Err("OCR motoru (ocr_engine veya ocr_engine.swift) bulunamadı.".to_string())
}

#[tauri::command]
pub async fn process_pdf_ocr(
    app: AppHandle,
    pdf_path: String,
    pages: Option<String>,
    languages: Option<String>,
    mode: Option<String>,
    extract_images: Option<bool>,
    output_dir: Option<String>,
    subfolder: Option<String>,
) -> Result<OCRResult, String> {
    let path = Path::new(&pdf_path);
    if !path.exists() {
        return Err(format!("Belirtilen PDF dosyası bulunamadı: {}", pdf_path));
    }

    crate::commands::image_ops::register_asset_search_dir(path);

    let target = resolve_ocr_target(&app)?;

    let page_arg = pages.unwrap_or_else(|| "all".to_string());
    let lang_arg = languages.unwrap_or_else(|| "tr-TR,en-US".to_string());
    let mode_arg = mode.unwrap_or_else(|| "hybrid".to_string());
    let extract_images_arg = if extract_images.unwrap_or(true) {
        "true".to_string()
    } else {
        "false".to_string()
    };
    let output_dir_arg = output_dir.unwrap_or_else(|| {
        path.parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string()
    });

    if !output_dir_arg.is_empty() {
        crate::commands::image_ops::register_asset_search_dir(Path::new(&output_dir_arg));
    }

    let mut common_args = vec![
        "--input".to_string(),
        pdf_path.clone(),
        "--pages".to_string(),
        page_arg,
        "--languages".to_string(),
        lang_arg,
        "--mode".to_string(),
        mode_arg,
        "--extract-images".to_string(),
        extract_images_arg,
    ];
    if !output_dir_arg.is_empty() {
        common_args.push("--output-dir".to_string());
        common_args.push(output_dir_arg);
    }
    if let Some(sub) = subfolder {
        if !sub.trim().is_empty() {
            common_args.push("--subfolder".to_string());
            common_args.push(sub);
        }
    }

    let mut cmd = match target {
        OCRExecutionTarget::CompiledBinary(bin_path) => {
            let mut c = Command::new(bin_path);
            c.args(&common_args);
            c
        }
        OCRExecutionTarget::SwiftScript(script_path) => {
            let mut c = Command::new("swift");
            c.arg(script_path);
            c.args(&common_args);
            c
        }
    };

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("OCR motoru başlatılamadı: {}", e))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "OCR standart çıktı kanalı alınamadı.".to_string())?;

    let mut reader = BufReader::new(stdout).lines();
    let mut final_result: Option<OCRResult> = None;

    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Try parsing progress event
        if let Ok(progress) = serde_json::from_str::<OCRProgressPayload>(trimmed) {
            if progress.msg_type == "progress" {
                let _ = app.emit("ocr:progress", progress);
            }
        }

        // Try parsing final result
        if let Ok(res) = serde_json::from_str::<OCRResult>(trimmed) {
            final_result = Some(res);
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("OCR süreci beklenirken hata oluştu: {}", e))?;

    if let Some(res) = final_result {
        if !res.success {
            return Err(res
                .error
                .unwrap_or_else(|| "Bilinmeyen bir OCR hatası oluştu.".to_string()));
        }
        Ok(res)
    } else {
        if !status.success() {
            Err(format!(
                "OCR motoru hata kodu ile sonlandı ({:?})",
                status.code()
            ))
        } else {
            Err("OCR işlemi tamamlandı ancak geçerli bir sonuç üretilemedi.".to_string())
        }
    }
}
