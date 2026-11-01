use crate::models::FileItem;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
                            !sub_name.starts_with('.') && sub_name != "target"
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
pub async fn read_file(file_path: String) -> Result<String, String> {
    crate::commands::image_ops::register_asset_search_dir(Path::new(&file_path));
    tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Dosya okunamadı: {}", e))
}

#[tauri::command]
pub async fn write_file(file_path: String, content: String) -> Result<bool, String> {
    crate::commands::image_ops::register_asset_search_dir(Path::new(&file_path));
    if let Some(parent) = Path::new(&file_path).parent()
        && !parent.exists()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Dizin oluşturulamadı: {}", e))?;
    }

    tokio::fs::write(&file_path, content)
        .await
        .map_err(|e| format!("Dosya yazılamadı: {}", e))?;
    Ok(true)
}

#[tauri::command]
pub async fn scan_directory(
    dir_path: String,
    markdown_only: Option<bool>,
) -> Result<Vec<FileItem>, String> {
    let path = PathBuf::from(dir_path);
    crate::commands::image_ops::register_asset_search_dir(&path);
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
    let has_ext = file_name.ends_with(".md")
        || file_name.ends_with(".mdx")
        || file_name.ends_with(".markdown")
        || file_name.ends_with(".txt");
    let final_name = if has_ext {
        file_name
    } else {
        format!("{}.md", file_name)
    };

    let target_path = Path::new(&dir_path).join(&final_name);
    if target_path.exists() {
        return Err("Bu isimde bir dosya zaten var.".to_string());
    }

    let stem = target_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Yeni Belge");
    let starter_content = format!("# {}\n\n", stem);

    tokio::fs::write(&target_path, starter_content)
        .await
        .map_err(|e| format!("Dosya oluşturulamadı: {}", e))?;

    Ok(target_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn create_directory(parent_path: String, dir_name: String) -> Result<String, String> {
    let target_path = Path::new(&parent_path).join(&dir_name);
    tokio::fs::create_dir_all(&target_path)
        .await
        .map_err(|e| format!("Klasör oluşturulamadı: {}", e))?;
    Ok(target_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn rename_file(old_path: String, new_path: String) -> Result<bool, String> {
    tokio::fs::rename(&old_path, &new_path)
        .await
        .map_err(|e| format!("Yeniden adlandırılamadı: {}", e))?;
    Ok(true)
}

#[tauri::command]
pub async fn delete_file(file_path: String) -> Result<bool, String> {
    let path_clone = file_path.clone();
    let trash_res = tokio::task::spawn_blocking(move || {
        trash::delete(Path::new(&path_clone))
    })
    .await
    .map_err(|e| e.to_string())?;

    if trash_res.is_err() {
        let path = Path::new(&file_path);
        if path.is_dir() {
            tokio::fs::remove_dir_all(path)
                .await
                .map_err(|e| format!("Silinemedi: {}", e))?;
        } else {
            tokio::fs::remove_file(path)
                .await
                .map_err(|e| format!("Silinemedi: {}", e))?;
        }
    }

    Ok(true)
}

#[tauri::command]
pub async fn get_available_path(base_dir: String, file_name: String) -> Result<String, String> {
    let base = Path::new(&base_dir);
    let mut candidate = base.join(&file_name);

    if !candidate.exists() {
        return Ok(candidate.to_string_lossy().to_string());
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
        let trimmed = stem.trim_end();
        if let Some(pos) = trimmed.rfind(|c: char| !c.is_ascii_digit()) {
            let (prefix, digits) = trimmed.split_at(pos + 1);
            if !digits.is_empty() {
                prefix.trim_end().to_string()
            } else {
                stem.clone()
            }
        } else {
            stem.clone()
        }
    };

    let mut counter = 2;
    loop {
        candidate = base.join(format!("{} {}{}", clean_stem, counter, ext));
        if !candidate.exists() {
            return Ok(candidate.to_string_lossy().to_string());
        }
        counter += 1;
        if counter > 1000 {
            return Ok(candidate.to_string_lossy().to_string());
        }
    }
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
