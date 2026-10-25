use serde_json::json;
use tauri::{Emitter, Manager};

/// Shows the main window, as the tray's *Show* does: it is rebuilt if
/// lightweight mode closed it.
#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "windows")]
        {
            let _ = window.set_skip_taskbar(false);
        }
        let _ = window.unminimize();
        window.show().map_err(|e| e.to_string())?;
        let _ = window.set_focus();
        Ok(())
    } else {
        crate::lightweight::exit_lightweight_mode(&app)
    }
}

/// Quits Switchy, as the tray's *Quit* does, a moment after answering so the
/// caller gets the reply.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        log::info!("Quitting the app (asked by command)");
        app.exit(0);
    });
}

/// The window's theme (`light`, `dark` or `system`) and language (`zh`, `en`
/// or `ja`), as Settings → General sets them. Either may be left out.
#[tauri::command]
pub fn set_window_appearance(
    app: tauri::AppHandle,
    theme: Option<String>,
    language: Option<String>,
) -> Result<(), String> {
    if let Some(theme) = theme.as_deref() {
        if !matches!(theme, "light" | "dark" | "system") {
            return Err(format!("Unknown theme '{theme}': light, dark or system"));
        }
    }
    if let Some(language) = language.as_deref() {
        if !matches!(language, "zh" | "en" | "ja") {
            return Err(format!("Unknown language '{language}': zh, en or ja"));
        }
        let mut settings = crate::settings::get_settings();
        settings.language = Some(language.to_string());
        crate::settings::update_settings(settings).map_err(|e| e.to_string())?;
        crate::tray::refresh_tray_menu(&app);
    }
    app.emit(
        "window-appearance",
        json!({ "theme": theme, "language": language }),
    )
    .map_err(|e| e.to_string())
}
