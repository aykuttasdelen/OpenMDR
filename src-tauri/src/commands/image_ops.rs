use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use std::path::Path;
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
