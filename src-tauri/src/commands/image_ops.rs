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

#[tauri::command]
pub async fn read_image_base64(
    file_path: String,
    base_dir: Option<String>,
) -> Result<String, String> {
    let mut clean_str = file_path.trim().to_string();

    // Handle file:// and asset:// prefixes if present
    if clean_str.starts_with("file://") {
        clean_str = clean_str.trim_start_matches("file://").to_string();
        if let Ok(decoded) = percent_encoding::percent_decode_str(&clean_str).decode_utf8() {
            clean_str = decoded.to_string();
        }
    } else if clean_str.starts_with("asset://localhost/") {
        clean_str = clean_str.trim_start_matches("asset://localhost/").to_string();
        if let Ok(decoded) = percent_encoding::percent_decode_str(&clean_str).decode_utf8() {
            clean_str = decoded.to_string();
        }
    }

    let mut path = PathBuf::from(&clean_str);

    // If path is relative, combine with base_dir if provided
    if !path.is_absolute() {
        if let Some(ref dir) = base_dir {
            if !dir.is_empty() {
                path = Path::new(dir).join(&clean_str);
            }
        }
    }

    if !path.exists() {
        return Err(format!("Görsel dosyası bulunamadı: {}", path.display()));
    }

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("Görsel okunamadı: {}", e))?;

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        _ => "image/png",
    };

    let encoded = BASE64.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, encoded))
}
