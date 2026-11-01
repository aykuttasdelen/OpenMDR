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
    pub error: Option<String>,
}

enum OCRExecutionTarget {
    CompiledBinary(PathBuf),
    SwiftScript(PathBuf),
}

fn resolve_ocr_target(app: &AppHandle) -> Result<OCRExecutionTarget, String> {
    // 1. Check bundle resources directory
    if let Ok(res_dir) = app.path().resource_dir() {
        let bin = res_dir.join("native/ocr_engine");
        if bin.exists() {
            return Ok(OCRExecutionTarget::CompiledBinary(bin));
        }
        let direct_bin = res_dir.join("ocr_engine");
        if direct_bin.exists() {
            return Ok(OCRExecutionTarget::CompiledBinary(direct_bin));
        }
        let script = res_dir.join("native/ocr_engine.swift");
        if script.exists() {
            return Ok(OCRExecutionTarget::SwiftScript(script));
        }
        let direct_script = res_dir.join("ocr_engine.swift");
        if direct_script.exists() {
            return Ok(OCRExecutionTarget::SwiftScript(direct_script));
        }
    }

    // 2. Check next to current executable
    if let Ok(curr_exe) = std::env::current_exe() {
        if let Some(parent) = curr_exe.parent() {
            let candidate1 = parent.join("ocr_engine");
            if candidate1.exists() {
                return Ok(OCRExecutionTarget::CompiledBinary(candidate1));
            }
            let candidate2 = parent.join("native/ocr_engine");
            if candidate2.exists() {
                return Ok(OCRExecutionTarget::CompiledBinary(candidate2));
            }
        }
    }

    // 3. Check workspace / relative directories
    let candidates = [
        "native/ocr_engine",
        "src-tauri/native/ocr_engine",
        "../src-tauri/native/ocr_engine",
    ];
    for c in &candidates {
        let path = Path::new(c);
        if path.exists() {
            return Ok(OCRExecutionTarget::CompiledBinary(path.to_path_buf()));
        }
    }

    let script_candidates = [
        "native/ocr_engine.swift",
        "src-tauri/native/ocr_engine.swift",
        "../src-tauri/native/ocr_engine.swift",
    ];
    for c in &script_candidates {
        let path = Path::new(c);
        if path.exists() {
            return Ok(OCRExecutionTarget::SwiftScript(path.to_path_buf()));
        }
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
) -> Result<OCRResult, String> {
    let path = Path::new(&pdf_path);
    if !path.exists() {
        return Err(format!("Belirtilen PDF dosyası bulunamadı: {}", pdf_path));
    }

    let target = resolve_ocr_target(&app)?;

    let page_arg = pages.unwrap_or_else(|| "all".to_string());
    let lang_arg = languages.unwrap_or_else(|| "tr-TR,en-US".to_string());
    let mode_arg = mode.unwrap_or_else(|| "hybrid".to_string());

    let mut cmd = match target {
        OCRExecutionTarget::CompiledBinary(bin_path) => {
            let mut c = Command::new(bin_path);
            c.args([
                "--input",
                &pdf_path,
                "--pages",
                &page_arg,
                "--languages",
                &lang_arg,
                "--mode",
                &mode_arg,
            ]);
            c
        }
        OCRExecutionTarget::SwiftScript(script_path) => {
            let mut c = Command::new("swift");
            c.arg(script_path);
            c.args([
                "--input",
                &pdf_path,
                "--pages",
                &page_arg,
                "--languages",
                &lang_arg,
                "--mode",
                &mode_arg,
            ]);
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
