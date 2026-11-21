pub mod commands;
pub mod history_store;
pub mod menu;
pub mod models;

use commands::{dialogs, export_ops, fs_ops, image_ops, ocr_ops};
use menu::{build_app_menu, handle_menu_event};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager, State, WindowEvent};

pub struct AppState {
    pub force_close: Arc<AtomicBool>,
}

#[tauri::command]
fn set_menu_language(app: tauri::AppHandle, lang: String) -> Result<(), String> {
    let is_en = lang == "en";
    let menu = build_app_menu(&app, is_en).map_err(|e| e.to_string())?;
    app.set_menu(menu).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn close_window(
    window: tauri::Window,
    state: State<AppState>,
    force: Option<bool>,
) -> Result<(), String> {
    if force.unwrap_or(false) {
        state.force_close.store(true, Ordering::SeqCst);
        let _ = window.close();
    } else {
        let _ = window.emit("app:window-closing", ());
    }
    Ok(())
}

#[tauri::command]
fn get_default_documents_dir(app: tauri::AppHandle) -> Result<String, String> {
    app.path()
        .document_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn toggle_maximize(window: tauri::Window) -> Result<(), String> {
    if let Ok(is_max) = window.is_maximized() {
        if is_max {
            let _ = window.unmaximize();
        } else {
            let _ = window.maximize();
        }
    }
    Ok(())
}

#[tauri::command]
fn set_window_theme(
    app: tauri::AppHandle,
    window: tauri::Window,
    theme: String,
) -> Result<(), String> {
    let tauri_theme = match theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    app.set_theme(tauri_theme);
    let _ = window.set_theme(tauri_theme);

    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{
            NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua,
            NSAppearanceNameDarkAqua, NSWindow,
        };
        let win_clone = window.clone();
        let theme_clone = theme.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(window_ptr) = win_clone.ns_window()
                && !window_ptr.is_null()
            {
                unsafe {
                    let ns_window = &*window_ptr.cast::<NSWindow>();
                    let appearance = match theme_clone.as_str() {
                        "light" => NSAppearance::appearanceNamed(NSAppearanceNameAqua),
                        "dark" => NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua),
                        _ => None,
                    };
                    ns_window.setAppearance(appearance.as_deref());
                }
            }
        });
    }

    Ok(())
}

#[tauri::command]
fn cancel_close(state: State<AppState>) -> Result<(), String> {
    state.force_close.store(false, Ordering::SeqCst);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let force_close = Arc::new(AtomicBool::new(false));
    let force_close_clone = Arc::clone(&force_close);

    tauri::Builder::default()
        .manage(AppState {
            force_close: Arc::clone(&force_close),
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle();
            let menu = build_app_menu(handle, false)?;
            app.set_menu(menu)?;

            // Scope asset protocol to user document directories; opened workspaces/files are added dynamically
            let asset_scope = app.asset_protocol_scope();
            for dir in [
                app.path().document_dir(),
                app.path().download_dir(),
                app.path().desktop_dir(),
                app.path().picture_dir(),
                app.path().temp_dir(),
            ]
            .into_iter()
            .flatten()
            {
                let _ = asset_scope.allow_directory(dir, true);
            }

            Ok(())
        })
        .on_menu_event(|app, event| {
            handle_menu_event(app, event.id().as_ref());
        })
        .on_window_event(move |window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && !force_close_clone.load(Ordering::SeqCst)
            {
                api.prevent_close();
                let _ = window.emit("app:window-closing", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            fs_ops::read_file,
            fs_ops::write_file,
            fs_ops::scan_directory,
            fs_ops::create_file,
            fs_ops::create_directory,
            fs_ops::rename_file,
            fs_ops::delete_file,
            fs_ops::get_available_path,
            fs_ops::show_in_folder,
            image_ops::save_image,
            image_ops::read_image_base64,
            dialogs::open_file_dialog,
            dialogs::open_pdf_dialog,
            dialogs::save_file_dialog,
            dialogs::open_directory_dialog,
            export_ops::save_html,
            ocr_ops::process_pdf_ocr,
            set_menu_language,
            get_default_documents_dir,
            toggle_maximize,
            set_window_theme,
            close_window,
            cancel_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_get_available_path() {
        let dir = tempdir().unwrap();
        let base = dir.path().to_str().unwrap().to_string();

        let path1 = fs_ops::get_available_path(base.clone(), "Test.md".to_string())
            .await
            .unwrap();
        assert!(path1.ends_with("Test.md"));

        // Create the file so it collides
        fs::write(&path1, "hello").unwrap();

        let path2 = fs_ops::get_available_path(base.clone(), "Test.md".to_string())
            .await
            .unwrap();
        assert!(path2.ends_with("Test 2.md"));

        fs::write(&path2, "hello 2").unwrap();

        let path3 = fs_ops::get_available_path(base.clone(), "Test.md".to_string())
            .await
            .unwrap();
        assert!(path3.ends_with("Test 3.md"));

        // If candidate already has numbering like "Test 2.md", it should properly increment to "Test 3.md"
        let path4 = fs_ops::get_available_path(base.clone(), "Test 2.md".to_string())
            .await
            .unwrap();
        assert!(path4.ends_with("Test 3.md"));

        // If candidate has numbering like "Test2.md" with Test2 created on disk
        fs::write(dir.path().join("Test2.md"), "hello test2").unwrap();
        let path5 = fs_ops::get_available_path(base, "Test2.md".to_string())
            .await
            .unwrap();
        assert!(path5.ends_with("Test 3.md"));
    }

    #[tokio::test]
    async fn test_save_image() {
        let dir = tempdir().unwrap();
        let base = dir.path().to_str().unwrap().to_string();

        // 1x1 transparent PNG in base64
        let b64 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let res = image_ops::save_image(base.clone(), b64.to_string(), "image/png".to_string())
            .await
            .unwrap();

        assert!(res.starts_with("assets/paste-"));
        assert!(res.ends_with(".png"));

        let file_path = dir.path().join(&res);
        assert!(file_path.exists());
        assert_eq!(fs::metadata(&file_path).unwrap().len(), 70);

        // Test resolve_and_read_image_base64 with relative path and base_dir
        let read_res =
            image_ops::resolve_and_read_image_base64(res.clone(), Some(base.clone()), vec![])
                .await
                .unwrap();
        assert!(read_res.starts_with("data:image/png;base64,"));

        // Test resolve_and_read_image_base64 with absolute path
        let abs_read = image_ops::resolve_and_read_image_base64(
            file_path.to_str().unwrap().to_string(),
            None,
            vec![],
        )
        .await
        .unwrap();
        assert!(abs_read.starts_with("data:image/png;base64,"));

        // Test subfolder asset resolution & percent-encoded Turkish characters (NFC/NFD/ASCII fallback)
        let sub_dir = dir.path().join("assets").join("Turkce_Belge_Ornegi");
        fs::create_dir_all(&sub_dir).unwrap();
        let sub_img = sub_dir.join("Turkce_Belge_Ornegi-p1-1.png");
        fs::copy(&file_path, &sub_img).unwrap();

        // 1) Requesting via percent-encoded Turkish path when disk file is ASCII-sanitized
        let encoded_tr = "assets/T%C3%BCrk%C3%A7e_Belge_%C3%96rne%C4%9Fi/T%C3%BCrk%C3%A7e_Belge_%C3%96rne%C4%9Fi-p1-1.png";
        let tr_res = image_ops::resolve_and_read_image_base64(
            encoded_tr.to_string(),
            Some(base.clone()),
            vec![],
        )
        .await
        .unwrap();
        assert!(tr_res.starts_with("data:image/png;base64,"));

        // 2) Requesting flat path (assets/Turkce_Belge_Ornegi-p1-1.png) when image is inside subfolder
        let flat_req = "assets/Turkce_Belge_Ornegi-p1-1.png";
        let flat_res = image_ops::resolve_and_read_image_base64(
            flat_req.to_string(),
            None,
            vec![dir.path().to_path_buf()],
        )
        .await
        .unwrap();
        assert!(flat_res.starts_with("data:image/png;base64,"));

        // 3) Requesting path with spaces/parentheses/hyphens when disk file is sanitized
        let punct_req = "assets/Türkçe - Belge (Örneği)/Türkçe - Belge (Örneği)-p1-1.png";
        let punct_res = image_ops::resolve_and_read_image_base64(
            punct_req.to_string(),
            Some(format!("/nonexistent/dir\n{}", base)),
            vec![],
        )
        .await
        .unwrap();
        assert!(punct_res.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn test_scan_dir_markdown_filtering() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // Create test markdown and non-markdown files
        fs::write(root.join("doc.md"), "# Title").unwrap();
        fs::write(root.join("note.mdx"), "# MDX").unwrap();
        fs::write(root.join("ignore.js"), "console.log()").unwrap();

        // Subfolder with markdown
        let sub_md = root.join("docs");
        fs::create_dir(&sub_md).unwrap();
        fs::write(sub_md.join("chapter.md"), "content").unwrap();

        // Subfolder with only non-markdown (should be pruned when markdown_only is true)
        let sub_img = root.join("images");
        fs::create_dir(&sub_img).unwrap();
        fs::write(sub_img.join("logo.png"), "fake png bytes").unwrap();

        // Truly empty subfolder (should be kept)
        let sub_empty = root.join("empty_folder");
        fs::create_dir(&sub_empty).unwrap();

        let items = fs_ops::scan_dir_recursive(root, 0, 4, true);

        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"docs"));
        assert!(names.contains(&"empty_folder"));
        assert!(!names.contains(&"images"), "images folder with no markdown must be pruned");
        assert!(names.contains(&"doc.md"));
        assert!(names.contains(&"note.mdx"));
        assert!(!names.contains(&"ignore.js"));
    }

    #[test]
    fn test_ocr_payload_parsing() {
        let sample_progress = r#"{"type":"progress","page":2,"totalPages":5,"percent":40}"#;
        let progress: ocr_ops::OCRProgressPayload = serde_json::from_str(sample_progress).unwrap();
        assert_eq!(progress.page, 2);
        assert_eq!(progress.total_pages, 5);
        assert_eq!(progress.percent, 40);

        // Backward compatibility: payload without extractedImages or images
        let sample_result = r##"{"type":"result","success":true,"totalPages":1,"processedPages":1,"markdown":"# Baslik\n\nIcerik","pages":[{"pageNumber":1,"isOcr":true,"text":"# Baslik\n\nIcerik"}],"error":null}"##;
        let result: ocr_ops::OCRResult = serde_json::from_str(sample_result).unwrap();
        assert!(result.success);
        assert_eq!(result.total_pages, 1);
        assert_eq!(result.pages.len(), 1);
        assert_eq!(result.pages[0].page_number, 1);
        assert!(result.pages[0].is_ocr);
        assert!(result.pages[0].images.is_empty());
        assert!(result.extracted_images.is_empty());
        assert_eq!(result.markdown, "# Baslik\n\nIcerik");

        // Payload with extractedImages and page images
        let sample_with_assets = r##"{"type":"result","success":true,"totalPages":2,"processedPages":2,"markdown":"# Page 1\n\n![Görsel](assets/img-p1-1.png)","pages":[{"pageNumber":1,"isOcr":false,"text":"# Page 1\n\n![Görsel](assets/img-p1-1.png)","images":["assets/img-p1-1.png"]},{"pageNumber":2,"isOcr":true,"text":"Page 2 text","images":[]}],"extractedImages":["assets/img-p1-1.png"],"error":null}"##;
        let result_assets: ocr_ops::OCRResult = serde_json::from_str(sample_with_assets).unwrap();
        assert!(result_assets.success);
        assert_eq!(result_assets.total_pages, 2);
        assert_eq!(result_assets.processed_pages, 2);
        assert_eq!(result_assets.extracted_images, vec!["assets/img-p1-1.png"]);
        assert_eq!(result_assets.pages[0].images, vec!["assets/img-p1-1.png"]);
        assert!(result_assets.pages[1].images.is_empty());
    }

    #[tokio::test]
    async fn test_fs_ops_security_and_utf8() {
        let dir = tempdir().unwrap();
        let base = dir.path().to_str().unwrap().to_string();

        assert!(fs_ops::create_file(base.clone(), "../escape.md".to_string()).await.is_err());
        assert!(fs_ops::create_file(base.clone(), "sub/file.md".to_string()).await.is_err());
        assert!(fs_ops::create_file(base.clone(), "..".to_string()).await.is_err());
        assert!(fs_ops::create_file(base.clone(), String::new()).await.is_err());

        assert!(fs_ops::create_directory(base.clone(), "../dir".to_string()).await.is_err());
        assert!(fs_ops::get_available_path(base.clone(), "../x.md".to_string()).await.is_err());

        let created_path = fs_ops::create_file(base.clone(), "CaseTest.MD".to_string())
            .await
            .unwrap();
        assert!(created_path.ends_with("CaseTest.MD"));
        let created_content = fs::read_to_string(&created_path).unwrap();
        assert!(created_content.contains("# CaseTest"));
        assert!(fs_ops::create_file(base.clone(), "CaseTest.MD".to_string()).await.is_err());

        let utf8_file = dir.path().join("Ö2.md");
        fs::write(&utf8_file, "test").unwrap();
        let available_path = fs_ops::get_available_path(base, "Ö2.md".to_string())
            .await
            .unwrap();
        assert!(available_path.ends_with("Ö 2.md"));
    }

    #[tokio::test]
    async fn test_image_ops_security() {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

        let dir = tempdir().unwrap();
        let base = dir.path().to_str().unwrap().to_string();

        let svg_b64 = BASE64.encode(b"<svg><script>alert(1)</script></svg>");
        assert!(
            image_ops::save_image(base.clone(), svg_b64, "image/svg+xml".to_string())
                .await
                .is_err()
        );

        assert!(
            image_ops::resolve_and_read_image_base64(
                "../secret.png".to_string(),
                Some(base.clone()),
                vec![]
            )
            .await
            .is_err()
        );

        let txt_path = dir.path().join("notes.txt");
        fs::write(&txt_path, "secret text").unwrap();
        assert!(
            image_ops::resolve_and_read_image_base64(
                "notes.txt".to_string(),
                Some(base.clone()),
                vec![]
            )
            .await
            .is_err()
        );

        let png_path = dir.path().join("ok.png");
        fs::write(&png_path, b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR").unwrap();
        let data_url = image_ops::resolve_and_read_image_base64(
            "ok.png".to_string(),
            Some(base),
            vec![],
        )
        .await
        .unwrap();
        assert!(data_url.starts_with("data:image/png;base64,"));
    }
}
