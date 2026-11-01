use crate::commands::fs_ops::scan_dir_recursive;
use crate::models::{OpenDirectoryResult, OpenFileResult};
use std::path::Path;

#[tauri::command]
pub async fn open_file_dialog() -> Result<Option<OpenFileResult>, String> {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Markdown Dosyası Aç")
        .add_filter("Markdown Belgeleri", &["md", "markdown", "mdx", "txt"])
        .add_filter("Tüm Dosyalar", &["*"]);

    let handle = dialog.pick_file().await;
    match handle {
        Some(file) => {
            let path = file.path().to_path_buf();
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| format!("Dosya okunamadı: {}", e))?;
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Belge.md")
                .to_string();
            Ok(Some(OpenFileResult {
                file_path: path.to_string_lossy().to_string(),
                content,
                name,
            }))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn open_pdf_dialog() -> Result<Option<String>, String> {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("PDF Dosyası Seç")
        .add_filter("PDF Belgeleri", &["pdf"])
        .add_filter("Tüm Dosyalar", &["*"]);

    let handle = dialog.pick_file().await;
    match handle {
        Some(file) => Ok(Some(file.path().to_string_lossy().to_string())),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn save_file_dialog(
    default_name: Option<String>,
    default_dir: Option<String>,
    content: Option<String>,
) -> Result<Option<String>, String> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title("Markdown Dosyasını Kaydet")
        .add_filter("Markdown Dosyası", &["md", "mdx"])
        .add_filter("Tüm Dosyalar", &["*"]);

    if let Some(ref name) = default_name {
        dialog = dialog.set_file_name(name);
    }
    if let Some(ref dir) = default_dir {
        dialog = dialog.set_directory(Path::new(dir));
    }

    let handle = dialog.save_file().await;
    match handle {
        Some(file) => {
            let path = file.path().to_path_buf();
            if let Some(ref text) = content {
                tokio::fs::write(&path, text)
                    .await
                    .map_err(|e| format!("Dosya yazılamadı: {}", e))?;
            }
            Ok(Some(path.to_string_lossy().to_string()))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn open_directory_dialog(
    markdown_only: Option<bool>,
) -> Result<Option<OpenDirectoryResult>, String> {
    let dialog = rfd::AsyncFileDialog::new().set_title("Klasör Aç");
    let handle = dialog.pick_folder().await;

    match handle {
        Some(folder) => {
            let dir_path = folder.path().to_path_buf();
            let path_clone = dir_path.clone();
            let md_only = markdown_only.unwrap_or(true);

            let tree = tokio::task::spawn_blocking(move || {
                scan_dir_recursive(&path_clone, 0, 4, md_only)
            })
            .await
            .map_err(|e| format!("Dizin tarama hatası: {}", e))?;

            Ok(Some(OpenDirectoryResult {
                dir_path: dir_path.to_string_lossy().to_string(),
                tree,
            }))
        }
        None => Ok(None),
    }
}
