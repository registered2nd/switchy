//! Tray menu
//!
//! Creates and updates the system tray icon and menu, and handles its events.

use tauri::menu::{CheckMenuItem, Menu, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::{Emitter, Manager};

use crate::app_config::AppType;
use crate::error::AppError;
use crate::store::AppState;

/// Tray menu labels, taken from the UI's own translation files under the
/// `tray` key so the tray speaks the same language as the window.
pub struct TrayTexts {
    pub show_main: String,
    pub no_providers_label: String,
    pub lightweight_mode: String,
    pub quit: String,
}

const LOCALE_EN: &str = include_str!("../../src/i18n/locales/en.json");
const LOCALE_ZH: &str = include_str!("../../src/i18n/locales/zh.json");
const LOCALE_JA: &str = include_str!("../../src/i18n/locales/ja.json");

fn tray_labels(locale_json: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(locale_json)
        .ok()
        .and_then(|v| v.get("tray").and_then(|t| t.as_object()).cloned())
        .unwrap_or_default()
}

impl TrayTexts {
    pub fn from_language(language: &str) -> Self {
        let local = tray_labels(match language {
            "zh" => LOCALE_ZH,
            "ja" => LOCALE_JA,
            _ => LOCALE_EN,
        });
        let english = tray_labels(LOCALE_EN);
        let pick = |key: &str, fallback: &str| {
            local
                .get(key)
                .or_else(|| english.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or(fallback)
                .to_string()
        };
        Self {
            show_main: pick("showMain", "Open main window"),
            no_providers_label: pick("noProviders", "(no providers)"),
            lightweight_mode: pick("lightweightMode", "Lightweight mode"),
            quit: pick("quit", "Quit"),
        }
    }
}

/// Per-app section of the tray menu
pub struct TrayAppSection {
    pub app_type: AppType,
    pub prefix: &'static str,
    pub empty_id: &'static str,
    pub header_label: &'static str,
    pub log_name: &'static str,
}

/// Suffix of the Auto menu item
pub const AUTO_SUFFIX: &str = "auto";

pub const TRAY_SECTIONS: [TrayAppSection; 4] = [
    TrayAppSection {
        app_type: AppType::Claude,
        prefix: "claude_",
        empty_id: "claude_empty",
        header_label: "Claude",
        log_name: "Claude",
    },
    TrayAppSection {
        app_type: AppType::Codex,
        prefix: "codex_",
        empty_id: "codex_empty",
        header_label: "Codex",
        log_name: "Codex",
    },
    TrayAppSection {
        app_type: AppType::Gemini,
        prefix: "gemini_",
        empty_id: "gemini_empty",
        header_label: "Gemini",
        log_name: "Gemini",
    },
    TrayAppSection {
        app_type: AppType::Kimi,
        prefix: "kimi_",
        empty_id: "kimi_empty",
        header_label: "Kimi",
        log_name: "Kimi",
    },
];

/// Sort providers: sort_index, then created_at, then name
fn sort_providers(
    providers: &indexmap::IndexMap<String, crate::provider::Provider>,
) -> Vec<(&String, &crate::provider::Provider)> {
    let mut sorted: Vec<_> = providers.iter().collect();
    sorted.sort_by(|(_, a), (_, b)| {
        match (a.sort_index, b.sort_index) {
            (Some(idx_a), Some(idx_b)) => return idx_a.cmp(&idx_b),
            (Some(_), None) => return std::cmp::Ordering::Less,
            (None, Some(_)) => return std::cmp::Ordering::Greater,
            _ => {}
        }

        match (a.created_at, b.created_at) {
            (Some(time_a), Some(time_b)) => return time_a.cmp(&time_b),
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            _ => {}
        }

        a.name.cmp(&b.name)
    });
    sorted
}

/// Handle a provider tray event
pub fn handle_provider_tray_event(app: &tauri::AppHandle, event_id: &str) -> bool {
    for section in TRAY_SECTIONS.iter() {
        if let Some(suffix) = event_id.strip_prefix(section.prefix) {
            // Auto clicked
            if suffix == AUTO_SUFFIX {
                log::info!("Switching {} to Auto mode", section.log_name);
                let app_handle = app.clone();
                let app_type = section.app_type.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(e) = handle_auto_click(&app_handle, &app_type) {
                        log::error!("Failed to switch {} to Auto mode: {e}", section.log_name);
                    }
                });
                return true;
            }

            // Provider clicked
            log::info!("Switching {} provider to: {suffix}", section.log_name);
            let app_handle = app.clone();
            let provider_id = suffix.to_string();
            let app_type = section.app_type.clone();
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = handle_provider_click(&app_handle, &app_type, &provider_id) {
                    log::error!("Failed to switch {} provider: {e}", section.log_name);
                }
            });
            return true;
        }
    }
    false
}

/// Handle an Auto click: enable the proxy and auto_failover
fn handle_auto_click(app: &tauri::AppHandle, app_type: &AppType) -> Result<(), AppError> {
    if let Some(app_state) = app.try_state::<AppState>() {
        let app_type_str = app_type.as_str();

        // Strict semantics: once Auto mode is on, switch to queue P1 immediately (P1, then P2, ...).
        // If the queue is empty, add the current provider as P1 so the user is not stuck unable to turn Auto on.
        let mut queue = app_state.db.get_failover_queue(app_type_str)?;
        if queue.is_empty() {
            let current_id =
                crate::settings::get_effective_current_provider(&app_state.db, app_type)?;
            let Some(current_id) = current_id else {
                return Err(AppError::Message(
                    "The failover queue is empty and no current provider is set; cannot enable Auto mode".to_string(),
                ));
            };
            app_state
                .db
                .add_to_failover_queue(app_type_str, &current_id)?;
            queue = app_state.db.get_failover_queue(app_type_str)?;
        }

        let p1_provider_id = queue
            .first()
            .map(|item| item.provider_id.clone())
            .ok_or_else(|| {
                AppError::Message(
                    "The failover queue is empty; cannot enable Auto mode".to_string(),
                )
            })?;

        // Actually enable failover: start the proxy, take over, turn on auto_failover
        let proxy_service = &app_state.proxy_service;

        // 1) Make sure the proxy is running (this sets proxy_enabled = true)
        let is_running = futures::executor::block_on(proxy_service.is_running());
        if !is_running {
            log::info!("[Tray] Auto mode: starting the proxy");
            if let Err(e) = futures::executor::block_on(proxy_service.start()) {
                log::error!("[Tray] Failed to start the proxy: {e}");
                return Err(AppError::Message(format!("Failed to start the proxy: {e}")));
            }
        }

        // 2) Take over the live config (so this app goes through the proxy)
        log::info!("[Tray] Auto mode: taking over {app_type_str}");
        if let Err(e) =
            futures::executor::block_on(proxy_service.set_takeover_for_app(app_type_str, true))
        {
            log::error!("[Tray] Takeover failed: {e}");
            return Err(AppError::Message(format!("Takeover failed: {e}")));
        }

        // 3) Set auto_failover_enabled = true
        app_state
            .db
            .set_proxy_flags_sync(app_type_str, true, true)?;

        // 3.1) Switch to queue P1 now (hot switch: no live write, only DB/settings/backup)
        if let Err(e) = futures::executor::block_on(
            proxy_service.switch_proxy_target(app_type_str, &p1_provider_id),
        ) {
            log::error!("[Tray] Auto mode: failed to switch to queue P1: {e}");
            return Err(AppError::Message(format!(
                "Auto mode: failed to switch to queue P1: {e}"
            )));
        }

        // 4) Update the tray menu
        if let Ok(new_menu) = create_tray_menu(app, app_state.inner()) {
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_menu(Some(new_menu));
            }
        }

        // 5) Emit events to the frontend
        let event_data = serde_json::json!({
            "appType": app_type_str,
            "proxyEnabled": true,
            "autoFailoverEnabled": true,
            "providerId": p1_provider_id
        });
        if let Err(e) = app.emit("proxy-flags-changed", event_data.clone()) {
            log::error!("Failed to emit proxy-flags-changed: {e}");
        }
        // Emit provider-switched (for backward compatibility; an Auto switch counts as a switch)
        if let Err(e) = app.emit("provider-switched", event_data) {
            log::error!("Failed to emit provider-switched: {e}");
        }
    }
    Ok(())
}

/// Handle a provider click: turn off auto_failover and switch provider
fn handle_provider_click(
    app: &tauri::AppHandle,
    app_type: &AppType,
    provider_id: &str,
) -> Result<(), AppError> {
    if let Some(app_state) = app.try_state::<AppState>() {
        let app_type_str = app_type.as_str();

        // Keep the proxy's enabled state; turn off only auto_failover
        let (proxy_enabled, _) = app_state.db.get_proxy_flags_sync(app_type_str);
        app_state
            .db
            .set_proxy_flags_sync(app_type_str, proxy_enabled, false)?;

        // Switch provider
        crate::commands::switch_provider(
            app_state.clone(),
            app_type_str.to_string(),
            provider_id.to_string(),
        )
        .map_err(AppError::Message)?;

        // Update the tray menu
        if let Ok(new_menu) = create_tray_menu(app, app_state.inner()) {
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_menu(Some(new_menu));
            }
        }

        // Emit events to the frontend
        let event_data = serde_json::json!({
            "appType": app_type_str,
            "proxyEnabled": proxy_enabled,
            "autoFailoverEnabled": false,
            "providerId": provider_id
        });
        if let Err(e) = app.emit("proxy-flags-changed", event_data.clone()) {
            log::error!("Failed to emit proxy-flags-changed: {e}");
        }
        // Emit provider-switched (for backward compatibility)
        if let Err(e) = app.emit("provider-switched", event_data) {
            log::error!("Failed to emit provider-switched: {e}");
        }
    }
    Ok(())
}

/// Build the dynamic tray menu
pub fn create_tray_menu(
    app: &tauri::AppHandle,
    app_state: &AppState,
) -> Result<Menu<tauri::Wry>, AppError> {
    let app_settings = crate::settings::get_settings();
    let tray_texts = TrayTexts::from_language(app_settings.language.as_deref().unwrap_or("en"));

    // Get visible apps setting, default to all visible
    let visible_apps = app_settings.visible_apps.unwrap_or_default();

    let mut menu_builder = MenuBuilder::new(app);

    // Top: open the main window
    let show_main_item =
        MenuItem::with_id(app, "show_main", &tray_texts.show_main, true, None::<&str>).map_err(
            |e| {
                AppError::Message(format!(
                    "Failed to create the open-main-window menu item: {e}"
                ))
            },
        )?;
    menu_builder = menu_builder.item(&show_main_item).separator();

    // One submenu per app type, so many providers do not make the menu too long
    for section in TRAY_SECTIONS.iter() {
        if !visible_apps.is_visible(&section.app_type) {
            continue;
        }

        let app_type_str = section.app_type.as_str();
        let providers = app_state.db.get_all_providers(app_type_str)?;

        let current_id =
            crate::settings::get_effective_current_provider(&app_state.db, &section.app_type)?
                .unwrap_or_default();

        if providers.is_empty() {
            // No providers: show a disabled item
            let label = format!("{} {}", section.header_label, tray_texts.no_providers_label);
            let empty_item = MenuItem::with_id(app, section.empty_id, &label, false, None::<&str>)
                .map_err(|e| {
                    AppError::Message(format!(
                        "Failed to create the {} empty placeholder: {e}",
                        section.log_name
                    ))
                })?;
            menu_builder = menu_builder.item(&empty_item);
        } else {
            // Providers present: build the submenu
            let current_name = providers.get(&current_id).map(|p| p.name.as_str());
            let submenu_label = match current_name {
                Some(name) => format!("{} · {}", section.header_label, name),
                None => section.header_label.to_string(),
            };
            let submenu_id = format!("submenu_{}", app_type_str);

            let mut submenu_builder = SubmenuBuilder::with_id(app, &submenu_id, &submenu_label);

            for (id, provider) in sort_providers(&providers) {
                let is_current = current_id == *id;
                let item = CheckMenuItem::with_id(
                    app,
                    format!("{}{}", section.prefix, id),
                    &provider.name,
                    true,
                    is_current,
                    None::<&str>,
                )
                .map_err(|e| {
                    AppError::Message(format!(
                        "Failed to create the {} menu item: {e}",
                        section.log_name
                    ))
                })?;
                submenu_builder = submenu_builder.item(&item);
            }

            let submenu = submenu_builder.build().map_err(|e| {
                AppError::Message(format!(
                    "Failed to build the {} submenu: {e}",
                    section.log_name
                ))
            })?;
            menu_builder = menu_builder.item(&submenu);
        }

        menu_builder = menu_builder.separator();
    }

    let lightweight_item = CheckMenuItem::with_id(
        app,
        "lightweight_mode",
        &tray_texts.lightweight_mode,
        true,
        crate::lightweight::is_lightweight_mode(),
        None::<&str>,
    )
    .map_err(|e| {
        AppError::Message(format!(
            "Failed to create the lightweight mode menu item: {e}"
        ))
    })?;

    menu_builder = menu_builder.item(&lightweight_item).separator();

    // Quit item (the separator was added in the section loop above)
    let quit_item = MenuItem::with_id(app, "quit", &tray_texts.quit, true, None::<&str>)
        .map_err(|e| AppError::Message(format!("Failed to create the quit menu item: {e}")))?;

    menu_builder = menu_builder.item(&quit_item);

    menu_builder
        .build()
        .map_err(|e| AppError::Message(format!("Failed to build the menu: {e}")))
}

pub fn refresh_tray_menu(app: &tauri::AppHandle) {
    use crate::store::AppState;

    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(new_menu) = create_tray_menu(app, state.inner()) {
            if let Some(tray) = app.tray_by_id("main") {
                if let Err(e) = tray.set_menu(Some(new_menu)) {
                    log::error!("Failed to refresh the tray menu: {e}");
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn apply_tray_policy(app: &tauri::AppHandle, dock_visible: bool) {
    use tauri::ActivationPolicy;

    let desired_policy = if dock_visible {
        ActivationPolicy::Regular
    } else {
        ActivationPolicy::Accessory
    };

    if let Err(err) = app.set_dock_visibility(dock_visible) {
        log::warn!("Failed to set Dock visibility: {err}");
    }

    if let Err(err) = app.set_activation_policy(desired_policy) {
        log::warn!("Failed to set the activation policy: {err}");
    }
}

/// Handle a tray menu event
pub fn handle_tray_menu_event(app: &tauri::AppHandle, event_id: &str) {
    log::info!("Handling tray menu event: {event_id}");

    match event_id {
        "show_main" => {
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "windows")]
                {
                    let _ = window.set_skip_taskbar(false);
                }
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
                #[cfg(target_os = "macos")]
                {
                    apply_tray_policy(app, true);
                }
            } else if crate::lightweight::is_lightweight_mode() {
                if let Err(e) = crate::lightweight::exit_lightweight_mode(app) {
                    log::error!("Failed to rebuild the window when leaving lightweight mode: {e}");
                }
            }
        }
        "lightweight_mode" => {
            if crate::lightweight::is_lightweight_mode() {
                if let Err(e) = crate::lightweight::exit_lightweight_mode(app) {
                    log::error!("Failed to exit lightweight mode: {e}");
                }
            } else if let Err(e) = crate::lightweight::enter_lightweight_mode(app) {
                log::error!("Failed to enter lightweight mode: {e}");
            }
        }
        "quit" => {
            log::info!("Quitting the app");
            app.exit(0);
        }
        _ => {
            if handle_provider_tray_event(app, event_id) {
                return;
            }
            log::warn!("Unhandled menu event: {event_id}");
        }
    }
}
