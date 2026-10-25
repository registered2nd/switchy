# Control routes audit, 2026-10-25

Every action in Switchy's window and tray, the command it ends in, and whether `POST /switchy/invoke` reached it before 1.0.45 and now. Paths in the source audit were relative to `src/`; "view only" rows change only what the window shows (a search filter, a text Format button, dismissing a banner, a polling interval) and have nothing to reach. A *composed* action is one the window builds a JSON argument for, or runs as a short sequence of commands; an agent sends the same object or sequence. A *dialog* action opens a native picker; its command that takes the path directly is named in the note.

Found and fixed in this pass:

- The window's theme lived only in the window's storage: `set_window_appearance` sets it (and the language, which a saved setting alone did not apply to the open window).
- The tray's *Show* and *Quit* had no command: `show_main_window` and `quit_app`.
- `exit_lightweight_mode` was refused while lightweight mode had closed the window, the only time it matters: the route now runs it, `show_main_window`, `is_lightweight_mode` and `quit_app` without the window.
- Adding a sign-in card, putting a plan account into a program and reading its quota were done by the window's code: `add_sign_in_account`, `use_account_in`, `remove_account_from` and `get_account_quota`.
- The company sidebar had left the Universal providers page and the OpenCode and OpenClaw imports without a control: they are back under *Add account* and on the OpenClaw page.

Still unreachable from any control, as before 1.0.45, though their commands are in the catalog: opening a terminal for a provider (`open_provider_terminal`), and the circuit-breaker settings and request-detail views, whose components are not used.

| Area | Action | Command(s) | Kind | Reachable before | Reachable now | Note |
|---|---|---|---|---|---|---|
| Company page: Add account | ▸ "<program> subscription" sign-in (Claude, ChatGPT, Google, Kimi Code, Cursor, Antigravity) | add_sign_in_account | direct | new in 1.0.45 | yes | {app, name?}; the person then signs in inside the program |
| Company page: Add account | ▸ "<plan> account" → Add | save_account | composed | new in 1.0.45 | yes | new Account {id: uuid, createdAt, company, kind:"plan", plan, region, name, secret}; plan and region ids exist only in config/companies.ts |
| Company page: Add account | ▸ Relays and gateways ▸ "For <program>…" → preset → Add | add_provider | composed | yes | yes | whole Provider built from a relay preset (config/*ProviderPresets.ts) |
| Company page: Add account | ▸ "Pay-as-you-go balance monitor" → Add | save_account | composed | new in 1.0.45 | yes | new Account {kind:"monitor", region, …} with a fresh uuid |
| Company page: Add account | ▸ "Import the <program> account signed in now" | import_default_config | direct | yes | yes | {app} |
| Company page: Kimi sign-in | "Add its console key…" → Add | save_account | composed | new in 1.0.45 | yes | new Account with loginProviderId = the Kimi Code provider id |
| Company page: Kimi sign-in | Plan key "Used in" chip (use / take out of OpenCode) | use_account_in / remove_account_from | direct | new in 1.0.45 | yes | {id, app} |
| Company page: Kimi sign-in | "Console key" → Save | save_account | direct | new in 1.0.45 | yes | edited get_accounts row |
| Account dialog | Name, Plan, Service (region), Key fields | (save_account) | composed | new in 1.0.45 | yes | fields of the Account sent on Save |
| Account dialog | "From the <company> console" link | open_external | direct | yes | yes | URL only in config/companies.ts |
| Plan account card | "Used in" chip → use in a program | use_account_in | direct | new in 1.0.45 | yes | {id, app}; adds it to the program and switches |
| Plan account card | OpenCode "Used in" chip again → take out | remove_account_from | direct | new in 1.0.45 | yes | OpenCode only |
| Plan account card | ⋯ ▸ Edit → Save | save_account | direct | new in 1.0.45 | yes | edited get_accounts row |
| Plan account card | ⋯ ▸ Open console | open_external | direct | yes | yes | URL only in config/companies.ts |
| Plan account card | "Usage in the console" (plan without a quota URL) | open_external | direct | yes | yes | URL only in config/companies.ts |
| Plan account card | ⋯ ▸ Delete → confirm | delete_account | direct | new in 1.0.45 | yes | {id}; also removes the providers it made |
| Balance monitor card | ⋯ ▸ Edit → Save | save_account | direct | new in 1.0.45 | yes | edited get_accounts row |
| Balance monitor card | ⋯ ▸ Open console | open_external | direct | yes | yes | URL only in config/companies.ts |
| Balance monitor card | ⋯ ▸ Delete → confirm | delete_account | direct | new in 1.0.45 | yes | {id} |
| Provider card | Enable (use this account) | switch_provider | direct | yes | yes | {app, id}; the window only adds a "needs the proxy" warning |
| Provider card | OpenCode / OpenClaw "Add" (into the program's config) | switch_provider | direct | yes | yes | additive apps: adds to the live config |
| Provider card | OpenCode / OpenClaw "Remove" → confirm | remove_provider_from_live_config | direct | yes | yes | {app, id}; the card stays |
| Provider card | OMO / OMO Slim "Enable", or click "In use" to turn it off | switch_provider / disable_current_omo / disable_current_omo_slim | direct | yes | yes |  |
| Provider card | OpenClaw "Set Default" | set_openclaw_default_model | composed | yes | yes | window builds {primary:"<id>/<models[0].id>", fallbacks:[rest]} from settingsConfig.models |
| Provider card | "Capture login" (Claude subscription card in use), confirm overwrite | capture_claude_account | direct | yes | yes | {providerId, force}; a needsConfirmation reply → repeat with force:true |
| Provider card | Lock in / Unlock (Claude, Codex subscription cards) | lock_account / unlock_account | direct | yes | yes | lock {app, id}; unlock {app} |
| Provider card | Edit (pencil) → Save | update_provider | composed | yes | yes | whole Provider: get_providers row with settingsConfig/meta edited; originalId when an OpenCode/OpenClaw key changes |
| Provider card | ⋯ ▸ Add to / Remove from the switching order | add_to_failover_queue / remove_from_failover_queue | direct | yes | yes | {appType, providerId}; shown only in failover mode |
| Provider card | ⋯ ▸ Duplicate | add_provider | composed | yes | yes | clone named "… copy" with a new id; for OpenCode/OpenClaw the key is "<id>-copy[-n]", checked against get_opencode/openclaw_live_provider_ids, with addToLive:false |
| Provider card | ⋯ ▸ Test model (confirm the first time) | stream_check_provider, reset_circuit_breaker, save_settings | composed | yes | yes | reset only when the check is operational or degraded; the first run saves streamCheckConfirmed:true |
| Provider card | ⋯ ▸ Delete → confirm | delete_provider | direct | yes | yes | {app, id}; not offered for the card in use |
| Provider card | Drag to reorder | update_providers_sort_order | direct | yes | yes | {app, updates:[{id, sortIndex}]} covering every card |
| Provider card | Website / base-URL line | open_external | direct | yes | yes | URL from websiteUrl or the config |
| Provider card | Usage badge refresh (usage script) | queryProviderUsage | direct | yes | yes | {providerId, app} |
| Provider card | Subscription quota refresh | get_subscription_quota / get_subscription_quota_for_provider | direct | yes | yes | {tool} for the live login; {tool, providerId} for a card's own login |
| Provider card | Search cards (Ctrl+F) | — | frontend-only | view only | view only | filters the shown list only |
| Provider form | Choose a preset, or Custom | (add_provider) | composed | yes | yes | fills the form from config/*ProviderPresets.ts; no command lists the presets |
| Provider form | Fields: name, notes, website, icon, key, endpoint, full-URL switch, models, API format, auth field, JSON/TOML/env editors | (add_provider / update_provider) | composed | yes | yes | assembled into settingsConfig + meta (C/providers/forms/ProviderForm.tsx:798) |
| Provider form | Claude quick toggles: hide attribution, teammates, tool search, effort high, disable auto-upgrade | (add/update_provider) | composed | yes | yes | edits the settingsConfig JSON |
| Provider form | Codex "1M context", auto-compact limit | (add/update_provider) | composed | yes | yes | edits the config TOML |
| Provider form | Advanced: model test, per-provider proxy (and its Clear), pricing overrides | (add/update_provider) | composed | yes | yes | meta.testConfig / proxyConfig / costMultiplier / pricingModelSource |
| Provider form | OpenCode / OpenClaw: add or remove models, model options, extra options | (add/update_provider) | composed | yes | yes | settingsConfig.models / options |
| Provider form | OMO: agent and category model pickers, advanced JSON, "Fill all recommended" | (add/update_provider) | composed | yes | yes | settingsConfig {agents, categories, otherFields} |
| Provider form | OMO "Import Local" | read_omo_local_file / read_omo_slim_local_file | direct | yes | yes | a read; fills the draft |
| Provider form | Add (submit) | add_provider (+ OpenClaw: get/set_openclaw_model_catalog, get/set_openclaw_default_model) | composed | yes | yes | window sets the id (uuid, OpenCode/OpenClaw key, or omo-<uuid>), createdAt, and meta.custom_endpoints from the preset's candidates; OpenClaw also merges the preset's models and sets a default model if none is set |
| Provider form | Save (edit submit) | update_provider | composed | yes | yes | the dialog opens on read_live_provider_settings / get_openclaw_live_provider for the card in use |
| Provider form | Fetch models (Claude, Codex, Gemini, Kimi, OpenCode, OpenClaw) | fetch_models_for_config | direct | yes | yes | {baseUrl, apiKey, isFullUrl?}; the result only fills a picker |
| Provider form | Copilot: Login with GitHub / Add another account / Retry | auth_start_login, auth_poll_for_account (loop), open_external, copy_text_to_clipboard | composed | yes | yes | start, then a person enters user_code at verification_uri, then poll {authProvider:"github_copilot", deviceCode} every 8 s or more |
| Provider form | Copilot: Cancel sign-in | — | frontend-only | view only | view only | stops the poll loop |
| Provider form | Copilot: verification link | — | frontend-only | view only | view only | plain `<a href>`; open_external takes the URL |
| Provider form | Copilot: Set default account | auth_set_default_account | direct | yes | yes | ids from auth_get_status / auth_list_accounts |
| Provider form | Copilot: Remove account | auth_remove_account | direct | yes | yes | {authProvider, accountId} |
| Provider form | Copilot: Logout all accounts | auth_logout | direct | yes | yes | {authProvider} |
| Provider form | Copilot: Select account | (add/update_provider), copilot_get_models_for_account | composed | yes | yes | sets meta.githubAccountId + authBinding; the model list reloads |
| Provider form | Endpoints: Test speed | test_api_endpoints | direct | yes | yes | {urls, timeoutSecs?} |
| Provider form | Endpoints: add or remove a URL, pick one, auto-select | (add/update_provider) | composed | yes | yes | new card: meta.custom_endpoints; picking sets the base URL; auto-select is meta.endpointAutoSelect |
| Provider form | Endpoints: Save (editing a card) | add_custom_endpoint / remove_custom_endpoint | composed | yes | yes | one call per URL added or removed, diffed against get_custom_endpoints |
| Provider form | "Use common config" checkbox (Claude, Codex, Gemini, Kimi) | (add/update_provider) | composed | yes | yes | merges the snippet into settingsConfig and sets meta.commonConfigEnabled |
| Provider form | Edit the common config snippet (Claude saves as you type; Codex/Gemini/Kimi on Save) | set_common_config_snippet | direct | yes | yes | {appType, snippet} |
| Provider form | Extract common config from the current config | extract_common_config_snippet, set_common_config_snippet | composed | yes | yes | extract {appType, settingsConfig}, then save the result |
| Provider form | Claude: Capture current account (confirm on change) | capture_claude_account | direct | yes | yes | force:true after the confirm |
| Provider form | Claude: Clear captured account → confirm | clear_claude_account | direct | yes | yes | {providerId} |
| Provider form | "Get the key" link | — | frontend-only | view only | view only | plain `<a href>`; open_external takes the URL |
| Provider form | JSON editor "Format" | — | frontend-only | view only | view only | reformats the text in place |
| Usage script dialog | Enable switch (confirm the first time) | save_settings | direct | yes | yes | first time saves usageConfirmed:true; the switch itself is meta.usage_script.enabled, sent on Save |
| Usage script dialog | Template buttons; script, key, URL, token, timeout, interval fields | (update_provider) | composed | yes | yes | meta.usage_script |
| Usage script dialog | Test script | testUsageScript / get_coding_plan_quota / copilot_get_usage(_for_account) | direct | yes | yes | depends on the template: custom/general/newapi, token plan, or GitHub Copilot |
| Usage script dialog | Format | — | frontend-only | view only | view only | reformats the script |
| Usage script dialog | Save | update_provider | composed | yes | yes | the Provider with meta.usage_script replaced |
| Startup / env banner | Env banner: Dismiss | — | frontend-only | view only | view only | sessionStorage env_banner_dismissed |
| Startup / env banner | Env banner: tick conflicts (one, all, clear) → Delete selected → confirm | delete_env_vars | direct | yes | yes | {conflicts} picked from check_env_conflicts {app}; returns the backup path |
| Startup / env banner | Config-load error dialog (OK quits) | plugin-dialog `message`, plugin-process `exit` | frontend-only | view only | view only | plugin calls; quit_app (registered, not in catalog) also quits |
| Settings ▸ General | Language: 中文 / English / 日本語 | save_settings | direct | yes | yes | `language` field; the window re-renders only while Settings is open. set_window_appearance (registered, not in catalog) re-renders it at any time |
| Settings ▸ General | Theme: light / dark / system | set_window_appearance | frontend-only | no | yes | theme: light, dark or system |
| Settings ▸ General | Sidebar companies: show / hide | save_settings | direct | yes | yes | hiddenCompanies |
| Settings ▸ General | Launch at login | save_settings, set_auto_launch | composed | yes | yes | launchOnStartup, plus {enabled} |
| Settings ▸ General | Silent startup | save_settings | direct | yes | yes | silentStartup |
| Settings ▸ General | Minimize to tray on close | save_settings | direct | yes | yes | minimizeToTrayOnClose |
| Settings ▸ General | Preferred terminal | save_settings | direct | yes | yes | preferredTerminal |
| Settings ▸ General | Manual sign-in | save_settings | direct | yes | yes | manualSignIn |
| Settings ▸ Programs | Proxy switch per program (shown when the local proxy feature is on) | set_proxy_takeover_for_app | direct | yes | yes | {appType, enabled} |
| Settings ▸ Programs | Switch-automatically per program (shown when the failover toggle is on) | set_auto_failover_enabled | direct | yes | yes | {appType, enabled} |
| Settings ▸ Programs | Oh My OpenCode profiles ▸ Add → preset → Add | add_provider | composed | yes | yes | Provider from the OMO / OMO Slim preset, id omo-<uuid> / omo-slim-<uuid> |
| Settings ▸ Programs | OpenClaw switch (show OpenClaw) | save_settings | direct | yes | yes | visibleApps.openclaw |
| Settings ▸ Pool | Proxy on / off (confirm the first start) | start_proxy_server / stop_proxy_with_restore (+ save_settings) | direct | yes | yes | the first start also saves proxyConfirmed:true |
| Settings ▸ Pool | Route through the proxy: Claude / Codex / Gemini | set_proxy_takeover_for_app | direct | yes | yes | {appType, enabled} |
| Settings ▸ Pool | Switch automatically: Claude / Codex / Gemini | set_auto_failover_enabled | direct | yes | yes | {appType, enabled} |
| Settings ▸ Pool | Account pool switches: rotate, require proxy, keep warm, Codex shared session | set_account_pool_config | direct | yes | yes | whole config, read with get_account_pool_config |
| Settings ▸ Pool | Account pool numbers: threshold %, keep-warm minutes (saved on blur) | set_account_pool_config | direct | yes | yes | 50–100; 15–1440 |
| Settings ▸ Pool | Codex terminal: Refresh / Refresh on exit / Cancel | refresh_codex_terminal / cancel_codex_terminal_refresh | direct | yes | yes | {socket} from get_codex_terminal_updates |
| Settings ▸ Pool | Local proxy ▸ Enable feature | save_settings | direct | yes | yes | enableLocalProxy |
| Settings ▸ Pool | Local proxy ▸ Logging switch | update_global_proxy_config | direct | yes | yes | enableLogging, on the get_global_proxy_config object |
| Settings ▸ Pool | Local proxy ▸ listen address / port → Save | update_global_proxy_config | direct | yes | yes | listenAddress; listenPort 1024–65535 |
| Settings ▸ Pool | Failover ▸ show the switch-automatically toggles (confirm the first time) | save_settings | direct | yes | yes | enableFailoverToggle (+ failoverConfirmed) |
| Settings ▸ Pool | Failover ▸ <app> queue: auto-failover switch | set_auto_failover_enabled | direct | yes | yes | {appType, enabled} |
| Settings ▸ Pool | Failover ▸ <app> queue: pick a provider → Add | add_to_failover_queue | direct | yes | yes | ids from get_available_providers_for_failover |
| Settings ▸ Pool | Failover ▸ <app> queue: remove an entry | remove_from_failover_queue | direct | yes | yes | {appType, providerId} |
| Settings ▸ Pool | Failover ▸ <app> settings → Save | update_proxy_config_for_app | direct | yes | yes | on the get_proxy_config_for_app object; the error rate is sent as 0–1 |
| Settings ▸ Pool | Failover ▸ <app> settings → Reset | — | frontend-only | view only | view only | reverts unsaved fields |
| Settings ▸ Pool | Rectifier: enabled, thinking signature, thinking budget | set_rectifier_config | direct | yes | yes | whole config, read with get_rectifier_config |
| Settings ▸ Pool | Optimizer: enabled, thinking optimizer, cache injection, cache TTL | set_optimizer_config | direct | yes | yes | whole config, read with get_optimizer_config |
| Settings ▸ Pool | Global outbound proxy: Scan | scan_local_proxies | direct | yes | yes |  |
| Settings ▸ Pool | Global proxy: Test | test_proxy_url | direct | yes | yes | {url} |
| Settings ▸ Pool | Global proxy: URL, username, password, Clear, pick a scanned proxy | (set_global_proxy_url) | composed | yes | yes | user:pass are folded into the URL; empty means a direct connection |
| Settings ▸ Pool | Global proxy: Save (button or Enter) | set_global_proxy_url | direct | yes | yes | {url} |
| Settings ▸ Data | Auto-backup interval | save_settings | direct | yes | yes | backupIntervalHours |
| Settings ▸ Data | Backups to keep | save_settings | direct | yes | yes | backupRetainCount |
| Settings ▸ Data | Create backup | create_db_backup | direct | yes | yes |  |
| Settings ▸ Data | Rename a backup | rename_db_backup | direct | yes | yes | {oldFilename, newName}; filenames from list_db_backups |
| Settings ▸ Data | Delete a backup → confirm | delete_db_backup | direct | yes | yes | {filename} |
| Settings ▸ Data | Restore a backup → confirm | restore_db_backup | direct | yes | yes | {filename}; returns a safety-backup id |
| Settings ▸ Data | Import: choose a file | open_file_dialog | dialog | yes | yes | import_config_from_file takes {filePath} |
| Settings ▸ Data | Import: Import | import_config_from_file, sync_current_providers_live | composed | yes | yes | the sync writes each program's current provider back |
| Settings ▸ Data | Import: clear the choice | — | frontend-only | view only | view only |  |
| Settings ▸ Data | Export | save_file_dialog, export_config_to_file | dialog | yes | yes | export_config_to_file takes {filePath} |
| Settings ▸ Advanced | Config dirs: type a path or Reset (app config, Claude, Claude mirror, Codex, Codex mirror, Gemini, Kimi, OpenCode) | (Save row) | composed | yes | yes | settings *ConfigDir / *MirrorConfigDir; the app dir goes to set_app_config_dir_override |
| Settings ▸ Advanced | Config dirs: Browse… | pick_directory | dialog | yes | yes | instead, put the path in save_settings (claudeConfigDir, …) or set_app_config_dir_override {path} |
| Settings ▸ Advanced | Save | save_settings, set_app_config_dir_override, set_auto_launch, sync_current_providers_live | composed | yes | yes | auto-launch only if it changed; sync only if a program dir changed |
| Settings ▸ Advanced | Restart now (after an app config dir change) | restart_app | direct | yes | yes |  |
| Settings ▸ Advanced | Provider test settings → Save | save_stream_check_config | direct | yes | yes | on the get_stream_check_config object |
| Settings ▸ Advanced | Logging: enabled, level | set_log_config | direct | yes | yes | {config:{enabled, level}} |
| Settings ▸ Advanced | Session repair: Scan | scan_broken_sessions | direct | yes | yes |  |
| Settings ▸ Advanced | Session repair: Repair (one session) | repair_broken_session | direct | yes | yes | {sourcePath} from the scan |
| Settings ▸ Advanced | Session repair: Repair all | repair_broken_session ×N, scan_broken_sessions | composed | yes | yes | one call per broken session |
| Settings ▸ Usage | Period: Today / 7 days / 30 days | get_usage_summary, get_usage_trends, get_provider_stats, get_model_stats, get_account_switches | direct | yes | yes | reads with startDate |
| Settings ▸ Usage | Auto-refresh interval (cycles 0 / 5 / 10 / 30 / 60 s) | — | frontend-only | view only | view only | polling period only |
| Settings ▸ Usage | "Turn on logging" | update_global_proxy_config | direct | yes | yes | enableLogging:true |
| Settings ▸ Usage | Request log: filters (app, status, provider, model, time) → Search | get_request_logs | direct | yes | yes | {filters, page, pageSize} |
| Settings ▸ Usage | Request log: Reset, Refresh, page buttons | get_request_logs | direct | yes | yes |  |
| Settings ▸ Usage | Pricing: per-app default multiplier and pricing source → Save | set_default_cost_multiplier ×3, set_pricing_model_source ×3 | composed | yes | yes | one pair each for claude, codex, gemini |
| Settings ▸ Usage | Pricing: Add model / Edit model → Save | update_model_pricing | direct | yes | yes | {modelId, displayName, inputCost, outputCost, cacheReadCost, cacheCreationCost} |
| Settings ▸ Usage | Pricing: Delete model → confirm | delete_model_pricing | direct | yes | yes | {modelId} |
| OpenClaw page | Add (header) → preset → Add | add_provider (+ get/set_openclaw_model_catalog, get/set_openclaw_default_model) | composed | yes | yes | see Provider form ▸ Add |
| OpenClaw page | Env: edit the JSON → Save | set_openclaw_env | direct | yes | yes | {env} |
| OpenClaw page | Tools: profile, allow and deny items → Save | set_openclaw_tools | direct | yes | yes | on the get_openclaw_tools object |
| OpenClaw page | Agents: primary and fallback models, workspace, timeout, context tokens, max concurrent → Save | set_openclaw_agents_defaults | direct | yes | yes | on the get_openclaw_agents_defaults object; model ids from the OpenClaw providers |
| OpenClaw workspace | Open the workspace folder | open_workspace_directory | direct | yes | yes | {subdir:"workspace"}; opens Explorer |
| OpenClaw workspace | Edit a workspace file → Save | write_workspace_file | direct | yes | yes | {filename, content} |
| OpenClaw workspace | Daily memory: Open folder | open_workspace_directory | direct | yes | yes | {subdir:"memory"} |
| OpenClaw workspace | Daily memory: Search | search_daily_memory_files | direct | yes | yes | {query} |
| OpenClaw workspace | Daily memory: Today, or edit a note → Save | write_daily_memory_file | direct | yes | yes | {filename:"YYYY-MM-DD.md", content}; "Today" alone writes nothing |
| OpenClaw workspace | Daily memory: Delete → confirm | delete_daily_memory_file | direct | yes | yes | {filename} |
| Universal providers (no nav control) | Card: Sync → confirm | sync_universal_provider | direct | yes | yes | {id} |
| Universal providers (no nav control) | Card: Edit → Save, or Save and sync | upsert_universal_provider (+ sync_universal_provider) | composed | yes | yes | whole UniversalProvider |
| Universal providers (no nav control) | Card: Delete → confirm | delete_universal_provider | direct | yes | yes | {id} |
| Universal providers (no nav control) | Form: preset, Claude / Codex / Gemini switches, JSON editors | (upsert_universal_provider) | composed | yes | yes | fields of the object |
| Clipboard (not counted) | Copy the proxy address; copy the Copilot device code (button, and automatically on sign-in start); "Copy" on the switch-failed toast | copy_text_to_clipboard (Copilot); navigator.clipboard (others) | — | yes | yes |  |
| Tray | Switch a program to an account | switch_provider | direct | yes | yes | {app, id} |
| Tray | Show the window | show_main_window | direct | no | yes | rebuilds the window in lightweight mode |
| Tray | Lightweight mode on / off | enter_lightweight_mode / exit_lightweight_mode | direct | on: yes; off: no | yes | off now runs while the window is closed |
| Tray | Quit | quit_app | direct | no | yes | answers, then quits |
| Companies page: Add account | ▸ One relay for Claude Code, Codex and Gemini CLI… | upsert_universal_provider, sync_universal_provider | composed | yes (no control after the company sidebar) | yes | the Universal providers page is reached from Add account again |
| Companies page: Add account | ▸ Import the providers in OpenCode's config | import_opencode_providers_from_live | direct | yes | yes |  |
| OpenClaw page | Import the providers in OpenClaw's config | import_openclaw_providers_from_live | direct | yes | yes |  |
| Claude card | Capture login | capture_claude_account | direct | yes | yes | {providerId, force?}; dispatched over the route, checked 2026-10-25 |
