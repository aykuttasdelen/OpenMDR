use crate::commands::history_ops::{
    capture_pre_write_baseline, record_write_with_baseline, HistoryState,
};
use crate::history_store::HistoryStore;
use crate::models::FileItem;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

pub(crate) fn validate_single_component(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("İsim boş olamaz.".to_string());
    }
    if name == "." || name == ".." {
        return Err("İsim '.' veya '..' olamaz.".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err("İsim '/', '\\' veya NUL karakteri içeremez.".to_string());
    }
    let mut components = Path::new(name).components();
    match components.next() {
        Some(Component::Normal(_)) => {}
        _ => return Err("Geçersiz dosya adı.".to_string()),
    }
    if components.next().is_some() {
        return Err("Geçersiz dosya adı.".to_string());
    }
    Ok(())
}

pub fn scan_dir_recursive(
    dir_path: &Path,
    depth: usize,
    max_depth: usize,
    markdown_only: bool,
) -> Vec<FileItem> {
    if depth > max_depth {
        return Vec::new();
    }

    let read_res = match fs::read_dir(dir_path) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut items = Vec::new();

    for entry in read_res.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files & heavy project directories
        if name.starts_with('.') && name != ".cursor" && name != ".agents" {
            continue;
        }
        if name == "node_modules" || name == "dist" || name == "dist-electron" || name == "target" {
            continue;
        }

        let full_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            let children = scan_dir_recursive(&full_path, depth + 1, max_depth, markdown_only);

            if markdown_only && children.is_empty() {
                // If directory has no markdown children, check if it had other non-markdown files.
                // If it only contained non-markdown files (assets, images, scripts), omit it.
                // If it is genuinely empty (0 files on disk), keep it so user can add files.
                if let Ok(mut sub_entries) = fs::read_dir(&full_path) {
                    let has_any_real_entry = sub_entries.any(|e| {
                        if let Ok(sub) = e {
                            let sub_name = sub.file_name().to_string_lossy().to_string();
                            let is_hidden =
                                sub_name.starts_with('.') && sub_name != ".cursor" && sub_name != ".agents";
                            !is_hidden
                                && sub_name != "node_modules"
                                && sub_name != "dist"
                                && sub_name != "dist-electron"
                                && sub_name != "target"
                        } else {
                            false
                        }
                    });
                    if has_any_real_entry {
                        continue;
                    }
                }
            }

            items.push(FileItem {
                name,
                path: full_path.to_string_lossy().to_string(),
                is_directory: true,
                children: Some(children),
                size: None,
            });
        } else {
            if markdown_only {
                let ext = full_path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if ext != "md" && ext != "mdx" {
                    continue;
                }
            }

            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            items.push(FileItem {
                name,
                path: full_path.to_string_lossy().to_string(),
                is_directory: false,
                children: None,
                size: Some(size),
            });
        }
    }

    // Sort: directories first, then alphabetically
    items.sort_by(|a, b| {
        if a.is_directory && !b.is_directory {
            std::cmp::Ordering::Less
        } else if !a.is_directory && b.is_directory {
            std::cmp::Ordering::Greater
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });

    items
}

#[tauri::command]
pub async fn read_file(app: tauri::AppHandle, file_path: String) -> Result<String, String> {
    let path = Path::new(&file_path);
    crate::commands::image_ops::register_asset_search_dir(path);
    crate::commands::image_ops::allow_asset_dir(&app, path);
    tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Dosya okunamadı: {}", e))
}

/// Writes `content` to `path`, capturing a pre-write baseline and recording a history snapshot.
pub(crate) async fn write_path_with_history(
    store: Option<Arc<HistoryStore>>,
    path: &Path,
    content: String,
    save_source: String,
) -> Result<(), String> {
    let pre_existing = capture_pre_write_baseline(store.clone(), path).await;

    tokio::fs::write(path, &content)
        .await
        .map_err(|e| format!("Dosya yazılamadı: {}", e))?;

    record_write_with_baseline(
        store,
        path.to_string_lossy().to_string(),
        pre_existing,
        content,
        save_source,
    )
    .await;

    Ok(())
}

/// Ensures parent directories exist, writes `content` to disk, and records version history.
pub(crate) async fn write_file_inner(
    store: Option<Arc<HistoryStore>>,
    file_path: String,
    content: String,
    save_source: Option<String>,
) -> Result<bool, String> {
    let path = Path::new(&file_path);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Dizin oluşturulamadı: {}", e))?;
    }

    let source = save_source.unwrap_or_else(|| "manual".to_string());
    write_path_with_history(store, path, content, source).await?;
    Ok(true)
}

#[tauri::command]
pub async fn write_file(
    app: tauri::AppHandle,
    history: tauri::State<'_, HistoryState>,
    file_path: String,
    content: String,
    save_source: Option<String>,
) -> Result<bool, String> {
    let path = Path::new(&file_path);
    crate::commands::image_ops::register_asset_search_dir(path);
    crate::commands::image_ops::allow_asset_dir(&app, path);
    write_file_inner(history.0.clone(), file_path, content, save_source).await
}

#[tauri::command]
pub async fn scan_directory(
    app: tauri::AppHandle,
    dir_path: String,
    markdown_only: Option<bool>,
) -> Result<Vec<FileItem>, String> {
    let path = PathBuf::from(dir_path);
    crate::commands::image_ops::register_asset_search_dir(&path);
    crate::commands::image_ops::allow_asset_dir(&app, &path);
    let md_only = markdown_only.unwrap_or(true);
    let items = tokio::task::spawn_blocking(move || {
        scan_dir_recursive(&path, 0, 4, md_only)
    })
    .await
    .map_err(|e| format!("Dizin tarama hatası: {}", e))?;

    Ok(items)
}

#[tauri::command]
pub async fn create_file(dir_path: String, file_name: String) -> Result<String, String> {
    validate_single_component(&file_name)?;

    let lower = file_name.to_lowercase();
    let has_ext = lower.ends_with(".md")
        || lower.ends_with(".mdx")
        || lower.ends_with(".markdown")
        || lower.ends_with(".txt");
    let final_name = if has_ext {
        file_name
    } else {
        format!("{}.md", file_name)
    };

    let target_path = Path::new(&dir_path).join(&final_name);

    let stem = target_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Yeni Belge");
    let starter_content = format!("# {}\n\n", stem);

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target_path)
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "Bu isimde bir dosya zaten var.".to_string()
            } else {
                format!("Dosya oluşturulamadı: {}", e)
            }
        })?;

    file.write_all(starter_content.as_bytes())
        .await
        .map_err(|e| format!("Dosya oluşturulamadı: {}", e))?;
    file.flush()
        .await
        .map_err(|e| format!("Dosya yazılamadı: {}", e))?;

    Ok(target_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn create_directory(parent_path: String, dir_name: String) -> Result<String, String> {
    validate_single_component(&dir_name)?;

    let target_path = Path::new(&parent_path).join(&dir_name);
    tokio::fs::create_dir_all(&target_path)
        .await
        .map_err(|e| format!("Klasör oluşturulamadı: {}", e))?;
    Ok(target_path.to_string_lossy().to_string())
}

/// Renames a path on disk and updates any tracked version history paths best-effort.
pub(crate) async fn rename_file_inner(
    store: Option<Arc<HistoryStore>>,
    old_path: String,
    new_path: String,
) -> Result<bool, String> {
    tokio::fs::rename(&old_path, &new_path)
        .await
        .map_err(|e| format!("Yeniden adlandırılamadı: {}", e))?;

    if let Some(store) = store {
        match tokio::task::spawn_blocking(move || store.handle_rename(&old_path, &new_path)).await
        {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                eprintln!("[history] Failed to update history paths on rename: {err}");
            }
            Err(err) => {
                eprintln!("[history] Rename history task failed: {err}");
            }
        }
    }

    Ok(true)
}

#[tauri::command]
pub async fn rename_file(
    history: tauri::State<'_, HistoryState>,
    old_path: String,
    new_path: String,
) -> Result<bool, String> {
    rename_file_inner(history.0.clone(), old_path, new_path).await
}

#[tauri::command]
pub async fn delete_file(file_path: String) -> Result<bool, String> {
    let path_clone = file_path.clone();
    tokio::task::spawn_blocking(move || trash::delete(Path::new(&path_clone)))
        .await
        .map_err(|e| format!("Silme görevi başarısız: {}", e))?
        .map_err(|e| format!("Çöp kutusuna taşınamadı: {}", e))?;

    Ok(true)
}

pub(crate) fn resolve_available_path(base_dir: &Path, file_name: &str) -> Result<PathBuf, String> {
    validate_single_component(file_name)?;

    let mut candidate = base_dir.join(file_name);

    if !candidate.exists() {
        return Ok(candidate);
    }

    let stem = candidate
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Belge")
        .to_string();
    let ext = candidate
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_else(|| ".md".to_string());

    // Clean stem from trailing numbering e.g. "İsimsiz 2" or "İsimsiz2" -> "İsimsiz"
    let clean_stem = {
        let result = stem
            .trim_end()
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .trim_end();
        if result.is_empty() {
            stem.clone()
        } else {
            result.to_string()
        }
    };

    let mut counter = 2;
    loop {
        candidate = base_dir.join(format!("{} {}{}", clean_stem, counter, ext));
        if !candidate.exists() {
            return Ok(candidate);
        }
        counter += 1;
        if counter > 1000 {
            return Err("Uygun dosya adı bulunamadı.".to_string());
        }
    }
}

#[tauri::command]
pub async fn get_available_path(base_dir: String, file_name: String) -> Result<String, String> {
    let candidate = resolve_available_path(Path::new(&base_dir), &file_name)?;
    Ok(candidate.to_string_lossy().to_string())
}

#[tauri::command]
pub fn show_in_folder(file_path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("-R")
            .arg(&file_path)
            .spawn()
            .map_err(|e| format!("Finder açılamadı: {}", e))?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let parent = Path::new(&file_path)
            .parent()
            .unwrap_or(Path::new(&file_path));
        Command::new("open")
            .arg(parent)
            .spawn()
            .map_err(|e| format!("Dizin açılamadı: {}", e))?;
    }
    Ok(())
}
