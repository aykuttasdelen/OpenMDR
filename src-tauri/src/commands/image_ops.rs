use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[tauri::command]
pub async fn save_image(
    base_dir: String,
    base64_data: String,
    mime_type: String,
) -> Result<String, String> {
    if base_dir.is_empty() || base64_data.is_empty() {
        return Err("Geçersiz parametreler: baseDir ve base64Data gereklidir.".to_string());
    }

    let assets_dir = Path::new(&base_dir).join("assets");
    tokio::fs::create_dir_all(&assets_dir)
        .await
        .map_err(|e| format!("Assets dizini oluşturulamadı: {}", e))?;

    let lower_mime = mime_type.to_lowercase();
    let ext = if lower_mime.contains("jpeg") || lower_mime.contains("jpg") {
        "jpg"
    } else if lower_mime.contains("gif") {
        "gif"
    } else if lower_mime.contains("webp") {
        "webp"
    } else if lower_mime.contains("svg") {
        "svg"
    } else {
        "png"
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let mut file_name = format!("paste-{}.{}", timestamp, ext);
    let mut file_path = assets_dir.join(&file_name);

    if file_path.exists() {
        file_name = format!("paste-{}-1.{}", timestamp, ext);
        file_path = assets_dir.join(&file_name);
    }

    let clean_base64 = if let Some(idx) = base64_data.find(";base64,") {
        &base64_data[idx + 8..]
    } else {
        &base64_data
    };

    let bytes = BASE64
        .decode(clean_base64.trim())
        .map_err(|e| format!("Base64 çözme hatası: {}", e))?;

    if bytes.is_empty() {
        return Err("Geçersiz görsel verisi.".to_string());
    }

    tokio::fs::write(&file_path, bytes)
        .await
        .map_err(|e| format!("Görsel kaydedilemedi: {}", e))?;

    Ok(format!("assets/{}", file_name))
}

use std::sync::{Mutex, OnceLock};
use tauri::Manager;

static REGISTERED_ASSET_DIRS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();

pub fn register_asset_search_dir(dir: &Path) {
    let dir_to_add = if dir.is_file() {
        dir.parent().map(|p| p.to_path_buf())
    } else {
        Some(dir.to_path_buf())
    };
    let Some(clean_dir) = dir_to_add else {
        return;
    };
    if clean_dir.as_os_str().is_empty() {
        return;
    }
    let mutex = REGISTERED_ASSET_DIRS.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut list) = mutex.lock() {
        if !list.contains(&clean_dir) {
            list.insert(0, clean_dir);
            if list.len() > 32 {
                list.pop();
            }
        }
    }
}

fn get_registered_asset_dirs() -> Vec<PathBuf> {
    if let Some(mutex) = REGISTERED_ASSET_DIRS.get()
        && let Ok(list) = mutex.lock()
    {
        return list.clone();
    }
    Vec::new()
}

fn percent_decode_repeatedly(input: &str) -> String {
    let mut current = input.to_string();
    for _ in 0..3 {
        if !current.contains('%') {
            break;
        }
        if let Ok(decoded) = percent_encoding::percent_decode_str(&current).decode_utf8() {
            let decoded_str = decoded.to_string();
            if decoded_str == current {
                break;
            }
            current = decoded_str;
        } else {
            break;
        }
    }
    current
}

pub fn fold_to_ascii_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_sep = false;

    for ch in s.chars() {
        if ('\u{0300}'..='\u{036F}').contains(&ch) {
            continue;
        }
        let mapped: Option<char> = match ch {
            'ç' | 'Ç' => Some('c'),
            'ğ' | 'Ğ' => Some('g'),
            'ı' | 'I' | 'İ' | 'i' => Some('i'),
            'ö' | 'Ö' => Some('o'),
            'ş' | 'Ş' => Some('s'),
            'ü' | 'Ü' => Some('u'),
            'â' | 'Â' | 'à' | 'á' | 'ä' => Some('a'),
            'ê' | 'Ê' | 'è' | 'é' | 'ë' => Some('e'),
            'î' | 'Î' | 'ì' | 'í' | 'ï' => Some('i'),
            'ô' | 'Ô' | 'ò' | 'ó' | 'õ' => Some('o'),
            'û' | 'Û' | 'ù' | 'ú' => Some('u'),
            c if c.is_ascii_alphanumeric() => Some(c.to_ascii_lowercase()),
            _ => None,
        };

        if let Some(c) = mapped {
            out.push(c);
            last_was_sep = false;
        } else if !out.is_empty() && !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }

    if out.ends_with('_') {
        out.pop();
    }
    out
}

fn find_in_base_dir(
    dir: &Path,
    rel_clean: &str,
    file_name: Option<&str>,
    target_ascii_key: Option<&str>,
) -> Option<PathBuf> {
    if !dir.exists() {
        return None;
    }

    // 1. Direct relative join (e.g. <dir>/assets/<subfolder>/<file>.png or <dir>/assets/<file>.png)
    let direct = dir.join(rel_clean);
    if direct.is_file() {
        return Some(direct);
    }

    let fname = file_name?;
    let assets_dir = dir.join("assets");
    if !assets_dir.is_dir() {
        return None;
    }

    // 2. Direct in <dir>/assets/<file_name>
    let flat_candidate = assets_dir.join(fname);
    if flat_candidate.is_file() {
        return Some(flat_candidate);
    }

    // Extract requested subfolder key if rel_clean is of the form assets/<subfolder>/<file>
    let requested_sub_key = Path::new(rel_clean)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .filter(|s| !s.eq_ignore_ascii_case("assets"))
        .map(fold_to_ascii_key);

    // 3. Search immediate subdirectories of <dir>/assets/*/<file_name> and ASCII-folded matches
    if let Ok(entries) = std::fs::read_dir(&assets_dir) {
        let mut sub_dirs = Vec::new();
        let mut flat_files = Vec::new();

        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                sub_dirs.push(p);
            } else if p.is_file() {
                flat_files.push(p);
            }
        }

        // Prioritize subdirectory whose ASCII-folded name matches the requested subfolder
        if let Some(ref req_sub) = requested_sub_key {
            sub_dirs.sort_by_key(|d| {
                let matches = d
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| fold_to_ascii_key(s) == *req_sub)
                    .unwrap_or(false);
                !matches
            });
        }

        let mut ascii_fallback: Option<PathBuf> = None;

        for entry_path in &sub_dirs {
            let exact_in_sub = entry_path.join(fname);
            if exact_in_sub.is_file() {
                return Some(exact_in_sub);
            }

            if let Some(target_key) = target_ascii_key
                && ascii_fallback.is_none()
                && let Ok(sub_entries) = std::fs::read_dir(entry_path)
            {
                for sub_entry in sub_entries.flatten() {
                    let sub_path = sub_entry.path();
                    if sub_path.is_file()
                        && let Some(name_str) = sub_path.file_name().and_then(|n| n.to_str())
                        && fold_to_ascii_key(name_str) == target_key
                    {
                        ascii_fallback = Some(sub_path);
                        break;
                    }
                }
            }
        }

        if ascii_fallback.is_some() {
            return ascii_fallback;
        }

        if let Some(target_key) = target_ascii_key {
            for file_path in flat_files {
                if let Some(name_str) = file_path.file_name().and_then(|n| n.to_str())
                    && fold_to_ascii_key(name_str) == target_key
                {
                    return Some(file_path);
                }
            }
        }
    }

    None
}

pub async fn resolve_and_read_image_base64(
    file_path: String,
    base_dir: Option<String>,
    fallback_dirs: Vec<PathBuf>,
) -> Result<String, String> {
    let mut clean_str = file_path
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string();

    if clean_str.is_empty() {
        return Err("Boş görsel yolu.".to_string());
    }

    let had_scheme = clean_str.starts_with("file://")
        || clean_str.starts_with("asset://")
        || clean_str.starts_with("http://asset.localhost/")
        || clean_str.starts_with("https://asset.localhost/");

    for prefix in [
        "file://localhost/",
        "file://",
        "asset://localhost/",
        "asset://",
        "http://asset.localhost/",
        "https://asset.localhost/",
    ] {
        if clean_str.starts_with(prefix) {
            clean_str = clean_str[prefix.len()..].to_string();
            break;
        }
    }

    clean_str = percent_decode_repeatedly(&clean_str).replace('\\', "/");

    if had_scheme
        && !clean_str.starts_with('/')
        && (clean_str.starts_with("Users/")
            || clean_str.starts_with("Volumes/")
            || clean_str.starts_with("private/")
            || clean_str.starts_with("tmp/")
            || clean_str.starts_with("var/"))
    {
        clean_str = format!("/{}", clean_str);
    }

    let mut resolved_path: Option<PathBuf> = None;

    // 1. Check if clean_str is already an existing absolute path
    let initial_path = PathBuf::from(&clean_str);
    if initial_path.is_absolute() && initial_path.is_file() {
        resolved_path = Some(initial_path);
    }

    // 2. Search across candidate base directories
    if resolved_path.is_none() {
        let rel_clean = clean_str
            .trim_start_matches("./")
            .trim_start_matches('/');
        let file_name = Path::new(rel_clean)
            .file_name()
            .and_then(|s| s.to_str());
        let target_ascii_key = file_name.map(fold_to_ascii_key);

        let mut candidate_dirs: Vec<PathBuf> = Vec::new();
        let mut push_dir = |p: PathBuf| {
            if !p.as_os_str().is_empty() && !candidate_dirs.contains(&p) {
                candidate_dirs.push(p);
            }
        };

        if let Some(ref raw_dirs) = base_dir {
            for part in raw_dirs.split(['\n', ';']) {
                let trimmed = percent_decode_repeatedly(part.trim());
                if !trimmed.is_empty() {
                    let p = PathBuf::from(&trimmed);
                    push_dir(p.clone());
                    if let Some(parent) = p.parent() {
                        push_dir(parent.to_path_buf());
                    }
                }
            }
        }

        for reg_dir in get_registered_asset_dirs() {
            push_dir(reg_dir.clone());
            if let Some(parent) = reg_dir.parent() {
                push_dir(parent.to_path_buf());
            }
        }

        for d in fallback_dirs {
            push_dir(d);
        }

        if let Ok(cwd) = std::env::current_dir() {
            push_dir(cwd.clone());
            if let Some(parent) = cwd.parent() {
                push_dir(parent.to_path_buf());
            }
        }

        for dir in &candidate_dirs {
            if let Some(found) = find_in_base_dir(
                dir,
                rel_clean,
                file_name,
                target_ascii_key.as_deref(),
            ) {
                resolved_path = Some(found);
                break;
            }
        }

        // 3. Fallback: check immediate subdirectories of candidate_dirs that have an `assets/` folder
        if resolved_path.is_none() {
            'outer: for dir in &candidate_dirs {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let sub_path = entry.path();
                    if !sub_path.is_dir() {
                        continue;
                    }
                    let Some(sub_name) = sub_path.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    if sub_name.starts_with('.')
                        || sub_name == "node_modules"
                        || sub_name == "target"
                        || sub_name == "Library"
                        || sub_name == "Applications"
                    {
                        continue;
                    }
                    if sub_path.join("assets").is_dir()
                        && let Some(found) = find_in_base_dir(
                            &sub_path,
                            rel_clean,
                            file_name,
                            target_ascii_key.as_deref(),
                        )
                    {
                        resolved_path = Some(found);
                        break 'outer;
                    }
                }
            }
        }
    }

    let path = resolved_path.ok_or_else(|| {
        format!(
            "Görsel dosyası bulunamadı: {} (base_dir: {:?})",
            clean_str, base_dir
        )
    })?;

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("Görsel okunamadı: {}", e))?;

    if bytes.is_empty() {
        return Err(format!("Görsel dosyası boş: {}", path.display()));
    }

    // Detect MIME from magic bytes first, then fallback to file extension
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "bmp" => "image/bmp",
            _ => "image/png",
        }
    };

    let encoded = BASE64.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, encoded))
}

#[tauri::command]
pub async fn read_image_base64(
    app: tauri::AppHandle,
    file_path: String,
    base_dir: Option<String>,
) -> Result<String, String> {
    let mut fallback_dirs = Vec::new();
    if let Ok(doc_dir) = app.path().document_dir() {
        fallback_dirs.push(doc_dir);
    }
    if let Ok(dl_dir) = app.path().download_dir() {
        fallback_dirs.push(dl_dir);
    }
    if let Ok(desk_dir) = app.path().desktop_dir() {
        fallback_dirs.push(desk_dir);
    }
    if let Ok(home_dir) = app.path().home_dir() {
        fallback_dirs.push(home_dir);
    }

    resolve_and_read_image_base64(file_path, base_dir, fallback_dirs).await
}
