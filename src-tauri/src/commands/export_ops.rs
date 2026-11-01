#[tauri::command]
pub async fn save_html(
    html_content: String,
    default_name: Option<String>,
    default_dir: Option<String>,
) -> Result<Option<String>, String> {
    let name = default_name.unwrap_or_else(|| "Belge.html".to_string());
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title("HTML Olarak Kaydet")
        .set_file_name(&name)
        .add_filter("HTML Belgesi", &["html", "htm"])
        .add_filter("Tüm Dosyalar", &["*"]);

    if let Some(dir) = default_dir {
        if !dir.is_empty() {
            dialog = dialog.set_directory(std::path::Path::new(&dir));
        }
    }

    if let Some(file) = dialog.save_file().await {
        let path = file.path().to_string_lossy().to_string();
        tokio::fs::write(file.path(), html_content)
            .await
            .map_err(|e| format!("HTML kaydedilemedi: {}", e))?;
        Ok(Some(path))
    } else {
        Ok(None)
    }
}
