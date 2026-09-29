# Codex model discovery in an open Switchy window

## Finding

The Worldbuilding Codex window is using a model catalog loaded before `gpt-6-sol` was added. Its WSL config selects `model_catalog_json = "context_models.json"`; [Codex's config reference](https://learn.chatgpt.com/docs/config-file/config-reference) says this file is loaded at startup. The installed Codex 0.159.0 binary already bundled Sol, but the custom catalog had nine entries and omitted it. Both Windows and WSL custom catalogs now contain the exact bundled Sol profile, with their other nine entries unchanged. This fixes discovery for new servers, not the already running Worldbuilding server.

Switchy contributes to the stale window's lifetime: its [Codex shell function](../src-tauri/src/proxy/codex_engine.rs) starts one app-server per interactive WSL window and keeps the remote client attached to it. On account changes, Switchy sends `account/login/start` to that server; its two-second loop does not reload the catalog or replace the server. Switchy's [proxy takeover](../src-tauri/src/services/proxy.rs) edits `openai_base_url` while preserving the catalog setting, and its [Codex backend GET handler](../src-tauri/src/proxy/handlers.rs) forwards model requests upstream without a model-list cache. The `models_cache.json` read in [keep-warm](../src-tauri/src/proxy/keep_warm.rs) selects a warm-up request model; it does not serve `model/list` or control worker spawning.

## Verification, 2026-09-29

| Check | Result |
| --- | --- |
| Live Worldbuilding client and server | `/proc/5213/cwd` is `/home/agentcode/Worldbuilding-1628SA`. Its remote client uses the Switchy socket for app-server PID 5132, started at 01:20:51 PDT. |
| Catalog timing | `/home/agentcode/.codex/context_models.json` was updated at 01:34:28 PDT, after that server started. `~/.codex/config.toml` still selects it. |
| Existing server, read-only `model/list` | Seven visible models; nine with `includeHidden: true`, exactly the old catalog's model IDs. Both omit `gpt-6-sol`. |
| Fresh temporary 0.159.0 server, same current config | Eight visible models, including `gpt-6-sol` with `low`, `medium`, `high`, `xhigh`, `max`, and `ultra`. Only the temporary process was stopped. |
| Catalog integrity | WSL and Windows files have the same SHA-256, `9ef69258f5d0343878b87af836ac9befad7dd00c18db34570ebfb5482359bacc`; all nine prior profiles are unchanged and Sol matches the installed binary's profile. |

The before/after catalogs and an earlier fresh-server response are in `/home/agentcode/alice_tmp/model_catalog_refresh_2026_09_29/`. The live socket comparison above was repeated directly against the running Worldbuilding server. It explains its reported `Unknown model` response. This investigation did not send a model request to OpenAI, so it does not establish account entitlement beyond discovery.

## Owner and smallest fix

Codex owns the catalog and its startup loading. Switchy owns the per-window app-server lifecycle it adds. For this window, finish any active turn, exit that Codex client, then resume the same thread in a new window so its new server loads the current catalog; leave other sessions and servers running. [Codex's app-server documentation](https://learn.chatgpt.com/docs/app-server) documents `thread/resume`, which preserves conversation history across server processes. An in-flight turn cannot continue uninterrupted through server replacement.

For future upgrades or custom-catalog edits, Switchy should detect that a window's server predates the installed binary or selected catalog and offer a per-window refresh at a safe point, preserving the thread ID for resume. After account changes, it should re-query `model/list` for account-specific availability, but that query alone cannot add a profile to a server that loaded an older startup catalog. Do not restart active servers automatically during a turn.
