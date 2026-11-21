pub mod commands;
pub mod drive_auth;
pub mod drive_client;
pub mod drive_sync;
pub mod history_store;
pub mod menu;
pub mod models;

use commands::history_ops::HistoryState;
use commands::{dialogs, drive_ops, export_ops, fs_ops, history_ops, image_ops, ocr_ops};
use drive_auth::{DriveAuthManager, KeychainTokenStore, OAuthEndpoints};
use drive_client::DriveClient;
use drive_sync::{DriveManager, DriveState};
use history_store::HistoryStore;
use menu::{build_app_menu, handle_menu_event};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager, RunEvent, State, WindowEvent};

pub struct AppState {
    pub force_close: Arc<AtomicBool>,
    pub skip_exit_flush: Arc<AtomicBool>,
}

fn normalize_compile_env(value: Option<&'static str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
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
    app: tauri::AppHandle,
    force: Option<bool>,
) -> Result<(), String> {
    if force.unwrap_or(false) {
        state.force_close.store(true, Ordering::SeqCst);
        state.skip_exit_flush.store(true, Ordering::SeqCst);
        app.exit(0);
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
    state.skip_exit_flush.store(false, Ordering::SeqCst);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let force_close = Arc::new(AtomicBool::new(false));
    let force_close_clone = Arc::clone(&force_close);
    let skip_exit_flush = Arc::new(AtomicBool::new(false));

    tauri::Builder::default()
        .manage(AppState {
            force_close: Arc::clone(&force_close),
            skip_exit_flush,
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle();
            let menu = build_app_menu(handle, false)?;
            app.set_menu(menu)?;

            let history_store = match app.path().app_local_data_dir() {
                Ok(data_dir) => {
                    let db_path = data_dir.join("history.db");
                    match HistoryStore::open(db_path) {
                        Ok(store) => Some(Arc::new(store)),
                        Err(err) => {
                            eprintln!("[history] Failed to open history store: {err}");
                            None
                        }
                    }
                }
                Err(err) => {
                    eprintln!("[history] Failed to resolve app_local_data_dir: {err}");
                    None
                }
            };

            let drive_manager = match history_store {
                Some(ref store) => match reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(60))
                    .build()
                {
                    Ok(http) => {
                        let built_in_client_id =
                            normalize_compile_env(option_env!("OPENMD_GOOGLE_CLIENT_ID"));
                        let built_in_client_secret =
                            normalize_compile_env(option_env!("OPENMD_GOOGLE_CLIENT_SECRET"));
                        let auth = Arc::new(
                            DriveAuthManager::new(
                                Arc::clone(store),
                                Arc::new(KeychainTokenStore),
                                http.clone(),
                                OAuthEndpoints::default(),
                            )
                            .with_built_in_credentials(
                                built_in_client_id,
                                built_in_client_secret,
                            ),
                        );
                        let client = DriveClient::new(http);
                        let home_dir = app.path().home_dir().ok();
                        let manager =
                            DriveManager::new(Arc::clone(store), auth, client, home_dir);
                        manager.set_app_handle(handle.clone());
                        {
                            let rt_handle = tauri::async_runtime::handle();
                            let _rt_guard = rt_handle.inner().enter();
                            manager.spawn_background_worker();
                        }
                        Some(manager)
                    }
                    Err(err) => {
                        eprintln!("[drive] Failed to build HTTP client: {err}");
                        None
                    }
                },
                None => None,
            };

            app.manage(HistoryState(history_store));
            app.manage(DriveState(drive_manager));

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
            history_ops::history_get_settings,
            history_ops::history_update_settings,
            history_ops::history_list_snapshots,
            history_ops::history_list_snapshots_by_file_id,
            history_ops::history_get_snapshot_content,
            history_ops::history_diff_snapshot,
            history_ops::history_restore_snapshot,
            history_ops::history_list_deleted_files,
            history_ops::history_recover_file,
            history_ops::history_delete_file_history,
            history_ops::history_clear_all,
            history_ops::history_get_stats,
            drive_ops::drive_get_status,
            drive_ops::drive_update_settings,
            drive_ops::drive_save_custom_oauth,
            drive_ops::drive_clear_custom_oauth,
            drive_ops::drive_connect,
            drive_ops::drive_cancel_auth,
            drive_ops::drive_disconnect,
            drive_ops::drive_sync_now,
            drive_ops::drive_flush_before_quit,
            drive_ops::drive_list_remote_files,
            drive_ops::drive_download_file_content,
            drive_ops::drive_restore_file,
            drive_ops::drive_restore_all,
            set_menu_language,
            get_default_documents_dir,
            toggle_maximize,
            set_window_theme,
            close_window,
            cancel_close,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            RunEvent::ExitRequested { api, .. } => {
                let force_close = app
                    .try_state::<AppState>()
                    .is_some_and(|s| s.force_close.load(Ordering::SeqCst));
                if !force_close && let Some(win) = app.get_webview_window("main") {
                    api.prevent_exit();
                    let _ = win.unminimize();
                    let _ = win.show();
                    let _ = win.set_focus();
                    let _ = win.emit("app:window-closing", ());
                }
            }
            RunEvent::Exit => {
                let skip_flush = app
                    .try_state::<AppState>()
                    .is_some_and(|s| s.skip_exit_flush.load(Ordering::SeqCst));
                if !skip_flush
                    && let Some(drive_state) = app.try_state::<DriveState>()
                    && let Some(manager) = drive_state.0.as_ref()
                {
                    manager.blocking_flush_on_terminate(std::time::Duration::from_secs(3));
                }
            }
            _ => {}
        });
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

        assert!(fs_ops::create_file_inner(base.clone(), "../escape.md".to_string(), None).await.is_err());
        assert!(fs_ops::create_file_inner(base.clone(), "sub/file.md".to_string(), None).await.is_err());
        assert!(fs_ops::create_file_inner(base.clone(), "..".to_string(), None).await.is_err());
        assert!(fs_ops::create_file_inner(base.clone(), String::new(), None).await.is_err());

        assert!(fs_ops::create_directory(base.clone(), "../dir".to_string()).await.is_err());
        assert!(fs_ops::get_available_path(base.clone(), "../x.md".to_string()).await.is_err());

        let created_path = fs_ops::create_file_inner(base.clone(), "CaseTest.MD".to_string(), None)
            .await
            .unwrap();
        assert!(created_path.ends_with("CaseTest.MD"));
        let created_content = fs::read_to_string(&created_path).unwrap();
        assert!(created_content.contains("# CaseTest"));
        assert!(fs_ops::create_file_inner(base.clone(), "CaseTest.MD".to_string(), None).await.is_err());

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

    #[tokio::test]
    async fn test_write_hook_captures_initial_baseline_then_manual_and_skips_non_utf8() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path).unwrap());

        // Case A: normal pre-existing UTF-8 file with no prior snapshots
        let existing_path = dir.path().join("existing.md");
        let existing_str = existing_path.to_string_lossy().to_string();
        fs::write(&existing_path, "original disk text").unwrap();

        let baseline =
            history_ops::capture_pre_write_baseline(Some(Arc::clone(&store)), &existing_path)
                .await;
        assert_eq!(baseline.as_deref(), Some("original disk text"));

        fs::write(&existing_path, "edited text").unwrap();
        history_ops::record_write_with_baseline(
            Some(Arc::clone(&store)),
            existing_str.clone(),
            baseline,
            "edited text".to_string(),
            "manual".to_string(),
        )
        .await;

        let snaps = store.list_snapshots(&existing_str).unwrap();
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].trigger, "manual");
        assert_eq!(
            store.get_snapshot_content(snaps[0].id).unwrap(),
            "edited text"
        );
        assert_eq!(snaps[1].trigger, "initial");
        assert_eq!(
            store.get_snapshot_content(snaps[1].id).unwrap(),
            "original disk text"
        );

        // Subsequent baseline capture skips files that already have snapshots
        let second_baseline =
            history_ops::capture_pre_write_baseline(Some(Arc::clone(&store)), &existing_path)
                .await;
        assert_eq!(second_baseline, None);

        // Case B: invalid UTF-8 on disk is skipped cleanly; manual snapshot still records
        let binary_path = dir.path().join("binary.md");
        let binary_str = binary_path.to_string_lossy().to_string();
        fs::write(&binary_path, [0xff_u8, 0xfe, 0x00]).unwrap();

        let bin_baseline =
            history_ops::capture_pre_write_baseline(Some(Arc::clone(&store)), &binary_path).await;
        assert_eq!(bin_baseline, None);

        fs::write(&binary_path, "clean utf8 content").unwrap();
        history_ops::record_write_with_baseline(
            Some(Arc::clone(&store)),
            binary_str.clone(),
            bin_baseline,
            "clean utf8 content".to_string(),
            "manual".to_string(),
        )
        .await;

        let bin_snaps = store.list_snapshots(&binary_str).unwrap();
        assert_eq!(bin_snaps.len(), 1);
        assert_eq!(bin_snaps[0].trigger, "manual");
        assert_eq!(
            store.get_snapshot_content(bin_snaps[0].id).unwrap(),
            "clean utf8 content"
        );

        // Case C: store == None completes cleanly without panic or error
        let none_path = dir.path().join("none_store.md");
        fs::write(&none_path, "prior").unwrap();
        assert_eq!(
            history_ops::capture_pre_write_baseline(None, &none_path).await,
            None
        );
        history_ops::record_write_with_baseline(
            None,
            none_path.to_string_lossy().to_string(),
            Some("prior".to_string()),
            "next".to_string(),
            "manual".to_string(),
        )
        .await;
    }

    #[tokio::test]
    async fn test_write_hook_autosave_throttles_without_losing_disk_write() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path).unwrap());

        store
            .update_settings(history_store::HistorySettings {
                enabled: true,
                max_versions_per_file: 50,
                autosave_interval_minutes: 5,
            })
            .unwrap();

        let doc_path = dir.path().join("autosave_doc.md");
        let doc_str = doc_path.to_string_lossy().to_string();

        // First write without save_source defaults to "manual"
        let first_ok = fs_ops::write_file_inner(
            Some(Arc::clone(&store)),
            doc_str.clone(),
            "v1 manual".to_string(),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(first_ok);
        assert_eq!(fs::read_to_string(&doc_path).unwrap(), "v1 manual");
        let snaps_after_first = store.list_snapshots(&doc_str).unwrap();
        assert_eq!(snaps_after_first.len(), 1);
        assert_eq!(snaps_after_first[0].trigger, "manual");

        // Immediate autosave must throttle snapshot creation while still writing to disk
        let auto_ok = fs_ops::write_file_inner(
            Some(Arc::clone(&store)),
            doc_str.clone(),
            "v2 autosaved".to_string(),
            Some("autosave".to_string()),
            None,
        )
        .await
        .unwrap();
        assert!(auto_ok);
        assert_eq!(fs::read_to_string(&doc_path).unwrap(), "v2 autosaved");

        let snaps_after_auto = store.list_snapshots(&doc_str).unwrap();
        assert_eq!(snaps_after_auto.len(), 1);
        assert_eq!(
            store.get_snapshot_content(snaps_after_auto[0].id).unwrap(),
            "v1 manual"
        );

        // store == None also writes to disk cleanly
        let no_history_path = dir.path().join("no_history.md");
        let no_history_str = no_history_path.to_string_lossy().to_string();
        assert!(
            fs_ops::write_file_inner(None, no_history_str, "disk only".to_string(), None, None)
                .await
                .unwrap()
        );
        assert_eq!(fs::read_to_string(&no_history_path).unwrap(), "disk only");
    }

    #[tokio::test]
    async fn test_rename_hook_updates_tracked_history() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path).unwrap());

        let old_path = dir.path().join("old_name.md");
        let new_path = dir.path().join("new_name.md");
        let old_str = old_path.to_string_lossy().to_string();
        let new_str = new_path.to_string_lossy().to_string();

        fs_ops::write_file_inner(
            Some(Arc::clone(&store)),
            old_str.clone(),
            "tracked content".to_string(),
            Some("manual".to_string()),
            None,
        )
        .await
        .unwrap();

        let renamed = fs_ops::rename_file_inner(
            Some(Arc::clone(&store)),
            old_str.clone(),
            new_str.clone(),
            None,
        )
        .await
        .unwrap();
        assert!(renamed);
        assert!(!old_path.exists());
        assert_eq!(fs::read_to_string(&new_path).unwrap(), "tracked content");
        assert!(store.list_snapshots(&old_str).unwrap().is_empty());
        let moved_snaps = store.list_snapshots(&new_str).unwrap();
        assert_eq!(moved_snaps.len(), 1);
        assert_eq!(
            store.get_snapshot_content(moved_snaps[0].id).unwrap(),
            "tracked content"
        );

        // store == None renames on disk cleanly
        let third_path = dir.path().join("third_name.md");
        let third_str = third_path.to_string_lossy().to_string();
        assert!(
            fs_ops::rename_file_inner(None, new_str, third_str, None)
                .await
                .unwrap()
        );
        assert!(!new_path.exists());
        assert_eq!(fs::read_to_string(&third_path).unwrap(), "tracked content");
    }
}
