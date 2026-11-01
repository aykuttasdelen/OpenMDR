#[tauri::command]
pub async fn save_html(
    html_content: String,
    default_name: Option<String>,
) -> Result<bool, String> {
    let name = default_name.unwrap_or_else(|| "Belge.html".to_string());
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("HTML Olarak Kaydet")
        .set_file_name(&name)
        .add_filter("HTML Belgesi", &["html", "htm"])
        .add_filter("Tüm Dosyalar", &["*"]);

    if let Some(file) = dialog.save_file().await {
        tokio::fs::write(file.path(), html_content)
            .await
            .map_err(|e| format!("HTML kaydedilemedi: {}", e))?;
        Ok(true)
    } else {
        Ok(false)
    }
}
