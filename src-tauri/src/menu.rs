use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Wry};

pub fn build_app_menu(app: &AppHandle, is_english: bool) -> Result<Menu<Wry>, tauri::Error> {
    let app_menu = Submenu::with_items(
        app,
        "OpenMDR",
        true,
        &[
            &PredefinedMenuItem::about(app, Some(if is_english { "About OpenMDR" } else { "OpenMDR Hakkında" }), None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "open-settings", if is_english { "Settings..." } else { "Ayarlar..." }, true, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, Some(if is_english { "Services" } else { "Hizmetler" }))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some(if is_english { "Hide OpenMDR" } else { "OpenMDR'yi Gizle" }))?,
            &PredefinedMenuItem::hide_others(app, Some(if is_english { "Hide Others" } else { "Diğerlerini Gizle" }))?,
            &PredefinedMenuItem::show_all(app, Some(if is_english { "Show All" } else { "Tümünü Göster" }))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some(if is_english { "Quit OpenMDR" } else { "OpenMDR'den Çık" }))?,
        ],
    )?;

    let file_menu = Submenu::with_items(
        app,
        if is_english { "File" } else { "Dosya" },
        true,
        &[
            &MenuItem::with_id(app, "new-file", if is_english { "New Document" } else { "Yeni Belge" }, true, Some("CmdOrCtrl+N"))?,
            &MenuItem::with_id(app, "open-file", if is_english { "Open File..." } else { "Dosya Aç..." }, true, Some("CmdOrCtrl+O"))?,
            &MenuItem::with_id(app, "open-folder", if is_english { "Open Folder..." } else { "Klasör Aç..." }, true, Some("CmdOrCtrl+Shift+O"))?,
            &MenuItem::with_id(app, "close-workspace", if is_english { "Close Workspace" } else { "Çalışma Alanını Kapat" }, true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "save-file", if is_english { "Save" } else { "Kaydet" }, true, Some("CmdOrCtrl+S"))?,
            &MenuItem::with_id(app, "save-file-as", if is_english { "Save As..." } else { "Farklı Kaydet..." }, true, Some("CmdOrCtrl+Shift+S"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "export-html", if is_english { "Export as HTML..." } else { "HTML Olarak Dışa Aktar..." }, true, None::<&str>)?,
            &MenuItem::with_id(app, "export-pdf", if is_english { "Export as PDF..." } else { "PDF Olarak Dışa Aktar..." }, true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "close-tab", if is_english { "Close Tab" } else { "Sekmeyi Kapat" }, true, Some("CmdOrCtrl+W"))?,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        app,
        if is_english { "Edit" } else { "Düzen" },
        true,
        &[
            &PredefinedMenuItem::undo(app, Some(if is_english { "Undo" } else { "Geri Al" }))?,
            &PredefinedMenuItem::redo(app, Some(if is_english { "Redo" } else { "Yinele" }))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some(if is_english { "Cut" } else { "Kes" }))?,
            &PredefinedMenuItem::copy(app, Some(if is_english { "Copy" } else { "Kopyala" }))?,
            &PredefinedMenuItem::paste(app, Some(if is_english { "Paste" } else { "Yapıştır" }))?,
            &PredefinedMenuItem::select_all(app, Some(if is_english { "Select All" } else { "Tümünü Seç" }))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "find", if is_english { "Find..." } else { "Bul..." }, true, Some("CmdOrCtrl+F"))?,
            &MenuItem::with_id(app, "command-palette", if is_english { "Command Palette..." } else { "Komut Paleti..." }, true, Some("CmdOrCtrl+K"))?,
        ],
    )?;

    let format_menu = Submenu::with_items(
        app,
        if is_english { "Format" } else { "Biçim" },
        true,
        &[
            &MenuItem::with_id(app, "format-bold", if is_english { "Bold" } else { "Kalın" }, true, Some("CmdOrCtrl+B"))?,
            &MenuItem::with_id(app, "format-italic", if is_english { "Italic" } else { "Eğik" }, true, Some("CmdOrCtrl+I"))?,
            &MenuItem::with_id(app, "format-link", if is_english { "Insert Link" } else { "Bağlantı Ekle" }, true, Some("CmdOrCtrl+Shift+K"))?,
            &MenuItem::with_id(app, "format-code", if is_english { "Code Block" } else { "Kod Bloğu" }, true, Some("CmdOrCtrl+Shift+C"))?,
            &MenuItem::with_id(app, "format-math", if is_english { "Math Formula" } else { "Matematik Formülü" }, true, Some("CmdOrCtrl+M"))?,
        ],
    )?;

    let view_menu = Submenu::with_items(
        app,
        if is_english { "View" } else { "Görünüm" },
        true,
        &[
            &MenuItem::with_id(app, "toggle-sidebar", if is_english { "Toggle Sidebar" } else { "Kenar Çubuğunu Aç/Kapat" }, true, Some("CmdOrCtrl+\\"))?,
            &MenuItem::with_id(app, "view-editor", if is_english { "Editor Only" } else { "Yalnızca Editör Modu" }, true, Some("CmdOrCtrl+1"))?,
            &MenuItem::with_id(app, "view-split", if is_english { "Split View" } else { "Bölünmüş Ekran (Editör & Önizleme)" }, true, Some("CmdOrCtrl+2"))?,
            &MenuItem::with_id(app, "view-preview", if is_english { "Preview Only" } else { "Yalnızca Önizleme Modu" }, true, Some("CmdOrCtrl+3"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, Some(if is_english { "Toggle Fullscreen" } else { "Tam Ekran" }))?,
        ],
    )?;

    let window_menu = Submenu::with_items(
        app,
        if is_english { "Window" } else { "Pencere" },
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some(if is_english { "Minimize" } else { "Simge Durumuna Küçült" }))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::bring_all_to_front(app, Some(if is_english { "Bring All to Front" } else { "Tümünü Öne Getir" }))?,
        ],
    )?;

    Menu::with_items(app, &[&app_menu, &file_menu, &edit_menu, &format_menu, &view_menu, &window_menu])
}

pub fn handle_menu_event(app: &AppHandle, event_id: &str) {
    let _ = app.emit("menu:command", event_id);
}
