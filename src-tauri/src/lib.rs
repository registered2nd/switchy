mod app_config;
mod app_store;
mod auto_launch;
mod codex_config;
mod commands;
mod config;
mod database;
mod error;
mod gemini_config;
mod init_status;
mod kimi_config;
mod lightweight;
mod openclaw_config;
mod opencode_config;
mod panic_hook;
mod paths;
mod provider;
mod provider_defaults;
mod proxy;
mod services;
mod session_repair;
mod settings;
mod store;

mod tray;
mod usage_script;

pub use app_config::AppType;
pub use codex_config::{get_codex_auth_path, get_codex_config_path, write_codex_live_atomic};
pub use commands::open_provider_terminal;
pub use commands::*;
pub use config::{get_claude_mcp_path, get_claude_settings_path, read_json_file};
pub use database::Database;
pub use error::AppError;
pub use kimi_config::{
    get_kimi_config_path, get_kimi_credentials_path, read_kimi_live, write_kimi_live_atomic,
};
pub use provider::{Provider, ProviderMeta};
pub use services::{EndpointLatency, ProviderService, ProxyService, SpeedtestService};
pub use settings::{update_settings, AppSettings};
pub use store::AppState;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use std::sync::Arc;
#[cfg(target_os = "macos")]
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;
use tauri::RunEvent;

/// Tauri command that updates the tray menu
#[tauri::command]
async fn update_tray_menu(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    match tray::create_tray_menu(&app, state.inner()) {
        Ok(new_menu) => {
            if let Some(tray) = app.tray_by_id("main") {
                tray.set_menu(Some(new_menu))
                    .map_err(|e| format!("Failed to update the tray menu: {e}"))?;
                return Ok(true);
            }
            Ok(false)
        }
        Err(err) => {
            log::error!("Failed to create the tray menu: {err}");
            Ok(false)
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_tray_icon() -> Option<Image<'static>> {
    const ICON_BYTES: &[u8] = include_bytes!("../icons/tray/macos/statusbar_template_3x.png");

    match Image::from_bytes(ICON_BYTES) {
        Ok(icon) => Some(icon),
        Err(err) => {
            log::warn!("Failed to load macOS tray icon: {err}");
            None
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before any HTTPS client is built. See `install_rustls_provider`.
    crate::proxy::http_client::install_rustls_provider();

    // Install the panic hook, which logs crashes to <app_config_dir>/crash.log (default ~/.switchy/crash.log)
    panic_hook::setup_panic_hook();

    let mut builder = tauri::Builder::default();

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if crate::lightweight::is_lightweight_mode() {
                if let Err(e) = crate::lightweight::exit_lightweight_mode(app) {
                    log::error!("Failed to rebuild the window when leaving lightweight mode: {e}");
                }
            }

            // Show and focus the window
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }));
    }

    let builder = builder
        // Intercept window close: minimise to the tray if the setting says so
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let settings = crate::settings::get_settings();

                if settings.minimize_to_tray_on_close {
                    api.prevent_close();
                    let _ = window.hide();
                    #[cfg(target_os = "windows")]
                    {
                        let _ = window.set_skip_taskbar(true);
                    }
                    #[cfg(target_os = "macos")]
                    {
                        tray::apply_tray_policy(window.app_handle(), false);
                    }
                } else {
                    window.app_handle().exit(0);
                }
            }
        })
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .setup(|app| {
            // Refresh the Store override first so later path lookups (logs, database, ...) are correct
            app_store::refresh_app_config_dir_override(app.handle());
            panic_hook::init_app_config_dir(crate::config::get_app_config_dir());

            // Initialise logging (a single file at <app_config_dir>/logs/switchy.log)
            {
                use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

                let log_dir = panic_hook::get_log_dir();

                // Make sure the log directory exists
                if let Err(e) = std::fs::create_dir_all(&log_dir) {
                    eprintln!("Failed to create the log directory: {e}");
                }

                // Delete the old log file at startup, so there is only ever one file
                let log_file_path = log_dir.join("switchy.log");
                let _ = std::fs::remove_file(&log_file_path);

                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        // Start at Trace so log::set_max_level() can adjust the level later
                        .level(log::LevelFilter::Trace)
                        .targets([
                            Target::new(TargetKind::Stdout),
                            Target::new(TargetKind::Folder {
                                path: log_dir,
                                file_name: Some("switchy".into()),
                            }),
                        ])
                        // Single-file mode: the old file is deleted at startup and rotated when it reaches the size limit
                        // Note: KeepSome(n) computes n-2 internally, so n=1 underflows usize
                        // KeepSome(2) is the smallest safe value and keeps no rotated files
                        .rotation_strategy(RotationStrategy::KeepSome(2))
                        // 1 GB file size limit
                        .max_file_size(1024 * 1024 * 1024)
                        .timezone_strategy(TimezoneStrategy::UseLocal)
                        .build(),
                )?;
            }

            // Initialise the database
            let app_config_dir = crate::config::get_app_config_dir();
            let db_path = app_config_dir.join(crate::paths::DB_FILE);
            // Create the database now (including schema migration)
            //
            // Users upgrading from v3.8.* usually hit the SQLite schema migration here.
            // If it fails (corrupt database, missing permissions, user_version too new, ...), tell the user clearly;
            // otherwise it just looks like "the app won't open / crashes".
            let db = loop {
                match crate::database::Database::init() {
                    Ok(db) => break Arc::new(db),
                    Err(e) => {
                        log::error!("Failed to init database: {e}");

                        if !show_database_init_error_dialog(app.handle(), &db_path, &e.to_string())
                        {
                            log::info!("User chose to exit");
                            std::process::exit(1);
                        }

                        log::info!("User chose to retry database initialisation");
                    }
                }
            };

            let app_state = AppState::new(db);

            // Give the proxy the AppHandle so failover can update the UI
            app_state.proxy_service.set_app_handle(app.handle().clone());

            // ============================================================
            // Per-table import logic (each kind of data is checked independently)
            // ============================================================

            // 2. OMO config import (from the local file when the database has no OMO provider)
            {
                let has_omo = app_state
                    .db
                    .get_all_providers("opencode")
                    .map(|providers| providers.values().any(|p| p.category.as_deref() == Some("omo")))
                    .unwrap_or(false);
                if !has_omo {
                    match crate::services::OmoService::import_from_local(&app_state, &crate::services::omo::STANDARD) {
                        Ok(provider) => {
                            log::info!("✓ Imported OMO config from local as provider '{}'", provider.name);
                        }
                        Err(AppError::OmoConfigNotFound) => {
                            log::debug!("○ No OMO config to import");
                        }
                        Err(e) => {
                            log::warn!("✗ Failed to import OMO config from local: {e}");
                        }
                    }
                }
            }

            // 2.3 OMO Slim config import (when no omo-slim provider in DB, import from local)
            {
                let has_omo_slim = app_state
                    .db
                    .get_all_providers("opencode")
                    .map(|providers| {
                        providers
                            .values()
                            .any(|p| p.category.as_deref() == Some("omo-slim"))
                    })
                    .unwrap_or(false);
                if !has_omo_slim {
                    match crate::services::OmoService::import_from_local(&app_state, &crate::services::omo::SLIM) {
                        Ok(provider) => {
                            log::info!(
                                "✓ Imported OMO Slim config from local as provider '{}'",
                                provider.name
                            );
                        }
                        Err(AppError::OmoConfigNotFound) => {
                            log::debug!("○ No OMO Slim config to import");
                        }
                        Err(e) => {
                            log::warn!("✗ Failed to import OMO Slim config from local: {e}");
                        }
                    }
                }
            }

            // Migrate the old app_config_dir setting to the Store
            if let Err(e) = app_store::migrate_app_config_dir_from_settings(app.handle()) {
                log::warn!("Failed to migrate app_config_dir: {e}");
            }

            // Startup no longer saves unconditionally, so user config is not overwritten by accident.

            // Build the dynamic tray menu
            let menu = tray::create_tray_menu(app.handle(), &app_state)?;

            // Build the tray
            let mut tray_builder = TrayIconBuilder::with_id("main")
                .on_tray_icon_event(|tray, event| match event {
                    // Left-click surfaces the main window (BACKLOG #7). Reuses
                    // the same show_main path as the menu item.
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } => {
                        tray::handle_tray_menu_event(tray.app_handle(), "show_main");
                    }
                    TrayIconEvent::Click { .. } => {}
                    _ => log::debug!("unhandled event {event:?}"),
                })
                .menu(&menu)
                .on_menu_event(|app, event| {
                    tray::handle_tray_menu_event(app, &event.id.0);
                })
                // Left-click is handled above; menu shows on right-click
                // (macOS secondary-click) via Tauri default.
                .show_menu_on_left_click(false);

            // Platform tray icon (macOS uses a template icon that adapts to light and dark)
            #[cfg(target_os = "macos")]
            {
                if let Some(icon) = macos_tray_icon() {
                    tray_builder = tray_builder.icon(icon).icon_as_template(true);
                } else if let Some(icon) = app.default_window_icon() {
                    log::warn!("Falling back to default window icon for tray");
                    tray_builder = tray_builder.icon(icon.clone());
                } else {
                    log::warn!("Failed to load macOS tray icon for tray");
                }
            }

            #[cfg(not(target_os = "macos"))]
            {
                if let Some(icon) = app.default_window_icon() {
                    tray_builder = tray_builder.icon(icon.clone());
                } else {
                    log::warn!("Failed to get default window icon for tray");
                }
            }

            let _tray = tray_builder.build(app)?;
            // Register this same instance as global state, so no second copy can drift out of step
            app.manage(app_state);

            // Load the log config from the database and apply it
            {
                let db = &app.state::<AppState>().db;
                if let Ok(log_config) = db.get_log_config() {
                    log::set_max_level(log_config.to_level_filter());
                    log::info!(
                        "Loaded log config: enabled={}, level={}",
                        log_config.enabled,
                        log_config.level
                    );
                }
            }

            // Initialise CopilotAuthManager
            {
                use crate::proxy::providers::copilot_auth::CopilotAuthManager;
                use commands::CopilotAuthState;
                use tokio::sync::RwLock;

                let app_config_dir = crate::config::get_app_config_dir();
                let copilot_auth_manager = CopilotAuthManager::new(app_config_dir);
                app.manage(CopilotAuthState(Arc::new(RwLock::new(copilot_auth_manager))));
                log::info!("✓ CopilotAuthManager initialized");
            }

            // Initialise the global outbound-proxy HTTP client
            {
                let db = &app.state::<AppState>().db;
                let proxy_url = db.get_global_proxy_url().ok().flatten();

                if let Err(e) = crate::proxy::http_client::init(proxy_url.as_deref()) {
                    log::error!(
                        "[GlobalProxy] [GP-005] Failed to initialize with saved config: {e}"
                    );

                    // Clear the invalid proxy config
                    if proxy_url.is_some() {
                        log::warn!(
                            "[GlobalProxy] [GP-006] Clearing invalid proxy config from database"
                        );
                        if let Err(clear_err) = db.set_global_proxy_url(None) {
                            log::error!(
                                "[GlobalProxy] [GP-007] Failed to clear invalid config: {clear_err}"
                            );
                        }
                    }

                    // Re-initialise with a direct connection
                    if let Err(fallback_err) = crate::proxy::http_client::init(None) {
                        log::error!(
                            "[GlobalProxy] [GP-008] Failed to initialize direct connection: {fallback_err}"
                        );
                    }
                }
            }

            // Crash recovery and automatic proxy-state restore
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<AppState>();

                // Any live backup means the last run may have exited abnormally while taken over
                let has_backups = match state.db.has_any_live_backup().await {
                    Ok(v) => v,
                    Err(e) => {
                        log::error!("Failed to check live backups: {e}");
                        false
                    }
                };
                // Check whether the live configs are still taken over (contain placeholders)
                let live_taken_over = state.proxy_service.detect_takeover_in_live_configs();

                if has_backups || live_taken_over {
                    log::warn!("Previous run exited abnormally (takeover leftovers found); restoring live configs...");
                    if let Err(e) = state.proxy_service.recover_from_crash().await {
                        log::error!("Failed to restore live configs: {e}");
                    } else {
                        log::info!("Live configs restored");
                    }
                }

                initialize_common_config_snippets(&state);

                // Restore the proxy from the proxy state saved in the settings table
                restore_proxy_state_on_startup(&state).await;

                // Keep pooled subscription accounts' session windows open, if
                // the user has asked for it. Off by default; the sweep reads
                // the switch itself and does nothing while it is off.
                crate::proxy::keep_warm::start(state.db.clone());

                // Periodic backup check (on startup)
                if let Err(e) = state.db.periodic_backup_if_needed() {
                    log::warn!("Periodic backup failed on startup: {e}");
                }

                // Periodic maintenance timer: run once per day while the app is running
                let db_for_timer = state.db.clone();
                tauri::async_runtime::spawn(async move {
                    const PERIODIC_MAINTENANCE_INTERVAL_SECS: u64 = 24 * 60 * 60;
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                        PERIODIC_MAINTENANCE_INTERVAL_SECS,
                    ));
                    interval.tick().await; // skip immediate first tick (already checked above)
                    loop {
                        interval.tick().await;
                        if let Err(e) = db_for_timer.periodic_backup_if_needed() {
                            log::warn!("Periodic maintenance timer failed: {e}");
                        }
                    }
                });
            });

            // Mirror the live Claude credentials file to the configured mirror
            // dir on every change. Keeps WSL (or any mirror target) downstream
            // of the rotating live refresh-token chain so the two sides don't
            // race each other to invalidate the shared refresh_token.
            services::credential_mirror::start();
            crate::proxy::codex_pool::set_app_handle(app.handle().clone());
            crate::proxy::codex_engine::start(app.handle().clone());
            services::credential_mirror::start_codex(app.state::<AppState>().db.clone());

            // Linux: disable WebKitGTK hardware acceleration so an EGL init failure cannot cause a white screen
            #[cfg(target_os = "linux")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.with_webview(|webview| {
                        use webkit2gtk::{WebViewExt, SettingsExt, HardwareAccelerationPolicy};
                        let wk_webview = webview.inner();
                        if let Some(settings) = WebViewExt::settings(&wk_webview) {
                            SettingsExt::set_hardware_acceleration_policy(&settings, HardwareAccelerationPolicy::Never);
                            log::info!("WebKitGTK hardware acceleration disabled");
                        }
                    });
                }
            }

            // Silent start: show the main window or not, per the setting
            let settings = crate::settings::get_settings();
            if let Some(window) = app.get_webview_window("main") {
                if settings.silent_startup {
                    // Silent start: keep the window hidden
                    let _ = window.hide();
                    #[cfg(target_os = "windows")]
                    let _ = window.set_skip_taskbar(true);
                    #[cfg(target_os = "macos")]
                    tray::apply_tray_policy(app.handle(), false);
                    log::info!("Silent start: main window hidden");
                } else {
                    // Normal start: show the window
                    let _ = window.show();
                    log::info!("Normal start: main window shown");
                }
            }


            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_providers,
            commands::get_current_provider,
            commands::add_provider,
            commands::update_provider,
            commands::delete_provider,
            commands::capture_claude_account,
            commands::clear_claude_account,
            commands::get_captured_claude_identity,
            commands::get_codex_account_identity,
            commands::remove_provider_from_live_config,
            commands::switch_provider,
            commands::import_default_config,
            commands::get_claude_config_status,
            commands::get_config_status,
            commands::get_claude_code_config_path,
            commands::get_default_claude_mirror_dir,
            commands::get_default_codex_mirror_dir,
            commands::get_config_dir,
            commands::open_config_folder,
            commands::pick_directory,
            commands::open_external,
            commands::get_init_error,
            commands::get_app_config_path,
            commands::open_app_config_folder,
            commands::get_claude_common_config_snippet,
            commands::set_claude_common_config_snippet,
            commands::get_common_config_snippet,
            commands::set_common_config_snippet,
            commands::extract_common_config_snippet,
            commands::read_live_provider_settings,
            commands::get_settings,
            commands::save_settings,
            commands::get_rectifier_config,
            commands::set_rectifier_config,
            commands::get_account_pool_config,
            commands::set_account_pool_config,
            commands::get_account_pool_quota,
            commands::get_optimizer_config,
            commands::set_optimizer_config,
            commands::get_copilot_optimizer_config,
            commands::set_copilot_optimizer_config,
            commands::get_log_config,
            commands::set_log_config,
            commands::restart_app,
            commands::copy_text_to_clipboard,
            // usage query
            commands::queryProviderUsage,
            commands::testUsageScript,
            // subscription quota
            commands::get_subscription_quota,
            commands::get_subscription_quota_for_provider,
            commands::get_coding_plan_quota,
            // model list fetch (OpenAI-compatible /v1/models)
            commands::fetch_models_for_config,
            // ours: endpoint speed test + custom endpoint management
            commands::test_api_endpoints,
            commands::get_custom_endpoints,
            commands::add_custom_endpoint,
            commands::remove_custom_endpoint,
            commands::update_endpoint_last_used,
            // app_config_dir override via Store
            commands::get_app_config_dir_override,
            commands::set_app_config_dir_override,
            // provider sort order management
            commands::update_providers_sort_order,
            // theirs: config import/export and dialogs
            commands::export_config_to_file,
            commands::import_config_from_file,
            commands::save_file_dialog,
            commands::open_file_dialog,
            commands::create_db_backup,
            commands::list_db_backups,
            commands::restore_db_backup,
            commands::rename_db_backup,
            commands::delete_db_backup,
            commands::sync_current_providers_live,
            // Deep link import
            update_tray_menu,
            // Environment variable management
            commands::check_env_conflicts,
            commands::delete_env_vars,
            commands::restore_env_backup,
            // Auto launch
            commands::set_auto_launch,
            commands::get_auto_launch_status,
            // Proxy server management
            commands::start_proxy_server,
            commands::stop_proxy_with_restore,
            commands::get_proxy_takeover_status,
            commands::set_proxy_takeover_for_app,
            commands::get_proxy_status,
            commands::get_proxy_config,
            commands::update_proxy_config,
            // Global & Per-App Config
            commands::get_global_proxy_config,
            commands::update_global_proxy_config,
            commands::get_proxy_config_for_app,
            commands::update_proxy_config_for_app,
            commands::get_default_cost_multiplier,
            commands::set_default_cost_multiplier,
            commands::get_pricing_model_source,
            commands::set_pricing_model_source,
            commands::is_proxy_running,
            commands::is_live_takeover_active,
            commands::switch_proxy_provider,
            // Proxy failover commands
            commands::get_provider_health,
            commands::reset_circuit_breaker,
            commands::get_circuit_breaker_config,
            commands::update_circuit_breaker_config,
            commands::get_circuit_breaker_stats,
            // Failover queue management
            commands::get_failover_queue,
            commands::get_available_providers_for_failover,
            commands::add_to_failover_queue,
            commands::remove_from_failover_queue,
            commands::get_auto_failover_enabled,
            commands::set_auto_failover_enabled,
            // Usage statistics
            commands::get_usage_summary,
            commands::get_usage_trends,
            commands::get_provider_stats,
            commands::get_account_switches,
            commands::get_model_stats,
            commands::get_request_logs,
            commands::get_request_detail,
            commands::get_model_pricing,
            commands::update_model_pricing,
            commands::delete_model_pricing,
            commands::check_provider_limits,
            // Stream health check
            commands::stream_check_provider,
            commands::stream_check_all_providers,
            commands::get_stream_check_config,
            commands::save_stream_check_config,
            // Session manager
            commands::scan_broken_sessions,
            commands::repair_broken_session,
            // Provider terminal
            commands::open_provider_terminal,
            // Universal Provider management
            commands::get_universal_providers,
            commands::get_universal_provider,
            commands::upsert_universal_provider,
            commands::delete_universal_provider,
            commands::sync_universal_provider,
            // OpenCode specific
            commands::import_opencode_providers_from_live,
            commands::get_opencode_live_provider_ids,
            // OpenClaw specific
            commands::import_openclaw_providers_from_live,
            commands::get_openclaw_live_provider_ids,
            commands::get_openclaw_live_provider,
            commands::scan_openclaw_config_health,
            commands::get_openclaw_default_model,
            commands::set_openclaw_default_model,
            commands::get_openclaw_model_catalog,
            commands::set_openclaw_model_catalog,
            commands::get_openclaw_agents_defaults,
            commands::set_openclaw_agents_defaults,
            commands::get_openclaw_env,
            commands::set_openclaw_env,
            commands::get_openclaw_tools,
            commands::set_openclaw_tools,
            // Global upstream proxy
            commands::get_global_proxy_url,
            commands::set_global_proxy_url,
            commands::test_proxy_url,
            commands::get_upstream_proxy_status,
            commands::scan_local_proxies,
            // Window theme control
            commands::set_window_theme,
            // Generic managed auth commands
            commands::auth_start_login,
            commands::auth_poll_for_account,
            commands::auth_list_accounts,
            commands::auth_get_status,
            commands::auth_remove_account,
            commands::auth_set_default_account,
            commands::auth_logout,
            // Copilot OAuth commands (multi-account support)
            commands::copilot_start_device_flow,
            commands::copilot_poll_for_auth,
            commands::copilot_poll_for_account,
            commands::copilot_list_accounts,
            commands::copilot_remove_account,
            commands::copilot_set_default_account,
            commands::copilot_get_auth_status,
            commands::copilot_logout,
            commands::copilot_is_authenticated,
            commands::copilot_get_token,
            commands::copilot_get_token_for_account,
            commands::copilot_get_models,
            commands::copilot_get_models_for_account,
            commands::copilot_get_usage,
            commands::copilot_get_usage_for_account,
            // OMO commands
            commands::read_omo_local_file,
            commands::get_current_omo_provider_id,
            commands::disable_current_omo,
            commands::read_omo_slim_local_file,
            commands::get_current_omo_slim_provider_id,
            commands::disable_current_omo_slim,
            // Workspace files (OpenClaw)
            commands::read_workspace_file,
            commands::write_workspace_file,
            // Daily memory files (OpenClaw workspace)
            commands::list_daily_memory_files,
            commands::read_daily_memory_file,
            commands::write_daily_memory_file,
            commands::delete_daily_memory_file,
            commands::search_daily_memory_files,
            commands::open_workspace_directory,
            // lightweight mode (for testing or low-resource environments)
            commands::enter_lightweight_mode,
            commands::exit_lightweight_mode,
            commands::is_lightweight_mode,
        ]);

    let app = builder
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, event| {
        // Handle exit requests (all platforms)
        if let RunEvent::ExitRequested { api, code, .. } = &event {
            // code None means the runtime triggered it (e.g. a hidden window's WebView was reclaimed and no window is left):
            // just prevent the exit and keep running in the tray.
            // code Some(_) means the user called app.exit() (e.g. the tray's Quit item):
            // clean up, then exit.
            if code.is_none() {
                log::info!("Runtime exit request (no live window); preventing exit to keep running in the tray");
                api.prevent_exit();
                return;
            }

            log::info!("User exit request (code={code:?}); cleaning up...");
            api.prevent_exit();

            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                cleanup_before_exit(&app_handle).await;
                log::info!("Cleanup done; exiting");

                // Wait briefly so all I/O (such as database writes) is flushed to disk
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                // std::process::exit avoids triggering ExitRequested again
                std::process::exit(0);
            });
            return;
        }

        #[cfg(target_os = "macos")]
        {
            match event {
                // macOS fires Reopen when the Dock icon reactivates the app; restore the main window here
                RunEvent::Reopen { .. } => {
                    if let Some(window) = app_handle.get_webview_window("main") {
                        #[cfg(target_os = "windows")]
                        {
                            let _ = window.set_skip_taskbar(false);
                        }
                        let _ = window.unminimize();
                        let _ = window.show();
                        let _ = window.set_focus();
                        tray::apply_tray_policy(app_handle, true);
                    } else if crate::lightweight::is_lightweight_mode() {
                        if let Err(e) = crate::lightweight::exit_lightweight_mode(app_handle) {
                            log::error!("Failed to rebuild the window when leaving lightweight mode: {e}");
                        }
                    }
                }
                // Handle opens triggered through the custom URL scheme (e.g. switchy://...)
                RunEvent::Opened { urls } => {
                    if let Some(url) = urls.first() {
                        let url_str = url.to_string();
                        log::info!("RunEvent::Opened with URL: {url_str}");

                        if url_str.starts_with("switchy://")
                            || url_str.starts_with("switchy://")
                        {
                            if crate::lightweight::is_lightweight_mode() {
                                if let Err(e) = crate::lightweight::exit_lightweight_mode(app_handle)
                                {
                                    log::error!("Failed to rebuild the window when leaving lightweight mode: {e}");
                                }
                            }

                            // Parse and broadcast the deep link event, with the same logic as single_instance
                            match crate::deeplink::parse_deeplink_url(&url_str) {
                                Ok(request) => {
                                    log::info!(
                                        "Successfully parsed deep link from RunEvent::Opened: resource={}, app={:?}",
                                        request.resource,
                                        request.app
                                    );

                                    if let Err(e) =
                                        app_handle.emit("deeplink-import", &request)
                                    {
                                        log::error!(
                                            "Failed to emit deep link event from RunEvent::Opened: {e}"
                                        );
                                    }
                                }
                                Err(e) => {
                                    log::error!(
                                        "Failed to parse deep link URL from RunEvent::Opened: {e}"
                                    );

                                    if let Err(emit_err) = app_handle.emit(
                                        "deeplink-error",
                                        serde_json::json!({
                                            "url": url_str,
                                            "error": e.to_string()
                                        }),
                                    ) {
                                        log::error!(
                                            "Failed to emit deep link error event from RunEvent::Opened: {emit_err}"
                                        );
                                    }
                                }
                            }

                            // Make sure the main window is visible
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.unminimize();
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app_handle, event);
        }
    });
}

// ============================================================
// Cleanup on exit
// ============================================================

/// Cleanup before the app exits
///
/// Checks the proxy server before exit; if it is running, stops it and restores the live configs,
/// so Claude Code/Codex/Gemini configs are never left broken.
/// stop_with_restore_keep_state keeps the proxy state in the settings table, so the next start restores it.
pub async fn cleanup_before_exit(app_handle: &tauri::AppHandle) {
    if let Some(state) = app_handle.try_state::<store::AppState>() {
        let proxy_service = &state.proxy_service;

        // Also a safety net on exit: the proxy may have crashed or stopped while takeover leftovers (placeholders/backups) remain.
        let has_backups = match state.db.has_any_live_backup().await {
            Ok(v) => v,
            Err(e) => {
                log::error!("Failed to check live backups on exit: {e}");
                false
            }
        };
        let live_taken_over = proxy_service.detect_takeover_in_live_configs();
        let needs_restore = has_backups || live_taken_over;

        if needs_restore {
            log::info!("Takeover leftovers found; restoring live configs (keeping proxy state)...");
            // The keep_state variant keeps the proxy state in the settings table
            if let Err(e) = proxy_service.stop_with_restore_keep_state().await {
                log::error!("Failed to restore live configs on exit: {e}");
            } else {
                log::info!(
                    "Live configs restored (proxy state kept; it is restored at next start)"
                );
            }
            return;
        }

        // Not taken over: just stop the proxy if it is running
        if proxy_service.is_running().await {
            log::info!("Proxy server is running; stopping it...");
            if let Err(e) = proxy_service.stop().await {
                log::error!("Failed to stop the proxy on exit: {e}");
            }
            log::info!("Proxy server cleanup done");
        }
    }
}

// ============================================================
// Restore proxy state at startup
// ============================================================

/// At startup, restore the proxy from the state saved in the proxy_config table
///
/// If `proxy_config.enabled` is `true` for any app, start the proxy
/// and take over that app's live config.
async fn restore_proxy_state_on_startup(state: &store::AppState) {
    // Collect the apps whose takeover must be restored (from proxy_config.enabled)
    let mut apps_to_restore = Vec::new();
    for app_type in ["claude", "codex", "gemini"] {
        if let Ok(config) = state.db.get_proxy_config_for_app(app_type).await {
            if config.enabled {
                apps_to_restore.push(app_type);
            }
        }
    }

    if apps_to_restore.is_empty() {
        log::debug!("No proxy state to restore at startup");
        return;
    }

    log::info!("Restoring proxy state from the previous run for: {apps_to_restore:?}");

    // Restore takeover one app at a time
    for app_type in apps_to_restore {
        match state
            .proxy_service
            .set_takeover_for_app(app_type, true)
            .await
        {
            Ok(()) => {
                log::info!("✓ Restored proxy takeover for {app_type}");
            }
            Err(e) => {
                log::error!("✗ Failed to restore proxy takeover for {app_type}: {e}");
                // On failure, clear this app's state so the next start does not try again
                if let Err(clear_err) = state
                    .proxy_service
                    .set_takeover_for_app(app_type, false)
                    .await
                {
                    log::error!("Failed to clear proxy state for {app_type}: {clear_err}");
                }
            }
        }
    }
}

fn initialize_common_config_snippets(state: &store::AppState) {
    // Auto-extract common config snippets from clean live files when snippet is missing.
    // This must run before proxy takeover is restored on startup, otherwise we'd read
    // proxy-placeholder configs instead of the user's actual live settings.
    for app_type in crate::app_config::AppType::all() {
        if !state
            .db
            .should_auto_extract_config_snippet(app_type.as_str())
            .unwrap_or(false)
        {
            continue;
        }

        let settings = match crate::services::provider::ProviderService::read_live_settings(
            app_type.clone(),
        ) {
            Ok(s) => s,
            Err(_) => continue,
        };

        match crate::services::provider::ProviderService::extract_common_config_snippet_from_settings(
            app_type.clone(),
            &settings,
        ) {
            Ok(snippet) if !snippet.is_empty() && snippet != "{}" => {
                match state.db.set_config_snippet(app_type.as_str(), Some(snippet)) {
                    Ok(()) => {
                        let _ = state.db.set_config_snippet_cleared(app_type.as_str(), false);
                        log::info!(
                            "✓ Auto-extracted common config snippet for {}",
                            app_type.as_str()
                        );
                    }
                    Err(e) => log::warn!(
                        "✗ Failed to save config snippet for {}: {e}",
                        app_type.as_str()
                    ),
                }
            }
            Ok(_) => log::debug!(
                "○ Live config for {} has no extractable common fields",
                app_type.as_str()
            ),
            Err(e) => log::warn!(
                "✗ Failed to extract config snippet for {}: {e}",
                app_type.as_str()
            ),
        }
    }

    let should_run_legacy_migration = state
        .db
        .is_legacy_common_config_migrated()
        .map(|done| !done)
        .unwrap_or(true);

    if should_run_legacy_migration {
        for app_type in [
            crate::app_config::AppType::Claude,
            crate::app_config::AppType::Codex,
            crate::app_config::AppType::Gemini,
            crate::app_config::AppType::Kimi,
        ] {
            if let Err(e) = crate::services::provider::ProviderService::migrate_legacy_common_config_usage_if_needed(
                state,
                app_type.clone(),
            ) {
                log::warn!(
                    "✗ Failed to migrate legacy common-config usage for {}: {e}",
                    app_type.as_str()
                );
            }
        }

        if let Err(e) = state.db.set_legacy_common_config_migrated(true) {
            log::warn!("✗ Failed to persist legacy common-config migration flag: {e}");
        }
    }
}

// ============================================================
// Database error dialog
// ============================================================

/// Show the database initialisation / schema migration failure dialog
/// Returns true if the user chose Retry, false if they chose Exit
fn show_database_init_error_dialog(
    app: &tauri::AppHandle,
    db_path: &std::path::Path,
    error: &str,
) -> bool {
    let title = "Database Initialization Failed";

    let message = format!(
        "An error occurred while initializing or migrating the database:\n\n{error}\n\n\
        Database file path:\n{db}\n\n\
        Your data is NOT lost - the app will not delete the database automatically.\n\
        Common causes include: newer database version, corrupted file, permission issues, or low disk space.\n\n\
        Suggestions:\n\
        1) Back up the entire config directory (including switchy.db)\n\
        2) If you see “database version is newer”, please upgrade Switchy\n\
        3) If this happened right after upgrading, consider rolling back to export/backup then upgrade again\n\n\
        Click 'Retry' to attempt initialization again\n\
        Click 'Exit' to close the program",
        db = db_path.display()
    );

    let retry_text = "Retry";
    let exit_text = "Exit";

    app.dialog()
        .message(&message)
        .title(title)
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::OkCancelCustom(
            retry_text.to_string(),
            exit_text.to_string(),
        ))
        .blocking_show()
}
