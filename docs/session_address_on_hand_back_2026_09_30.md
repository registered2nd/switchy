# How open sessions follow the API address when the proxy hands back

Measured 2026-09-30 on Claude Code 2.1.285 and Codex CLI 0.159.2 in WSL, each in a throwaway session with its own config directory, pointed at two dummy local listeners (ports 188x1 and 188x2) that log every request.

## Claude Code

The address comes from `env.ANTHROPIC_BASE_URL` in `settings.json`, which a running session watches.

| Change to `settings.json` mid-session | Next request went to |
|---|---|
| address set to listener 1 at start | listener 1 |
| changed to listener 2 | listener 2 |
| **removed** | **listener 2** (the last value it had) |
| set to `https://api.anthropic.com` | Anthropic (no dummy listener hit) |

A removed address is ignored. Before the fix, handing back a config by deleting the proxy address left every open session calling `127.0.0.1:15721`, which broke once the proxy stopped. The hand-back now writes Anthropic's address when the restored config names none, on Windows and on the WSL mirror (`name_claude_base_url` in `src-tauri/src/services/proxy.rs`). Confirmed live the same day: after Claude routing was turned off at 04:39:13, the proxy logged no further Claude requests while the open session kept answering.

## Codex

Switchy points Codex at the proxy with the top-level `openai_base_url`. Current Codex runs a conversation through a background app-server daemon.

| Change to `config.toml` mid-session | Requests |
|---|---|
| `openai_base_url` = listener 1 at start | all to listener 1 |
| changed to listener 2 | the turn's request to **listener 1**; side connections and one request to listener 2 |
| **removed** | back to **listener 1**, the address the window started with |
| changed to listener 2, listener 1 stopped | the turn retried listener 1 and sat at "Reconnecting… waiting for network" |

A Codex window keeps the address it started with, whatever the config says. A window opened under the proxy therefore needs the proxy's port answered for as long as it stays open. Turning the proxy off now hands the configs back but leaves the listener up, reported as off, until the proxy is turned on again or Switchy quits (`release` in `services/proxy.rs`). Quitting still breaks open Codex windows; only a proxy process that outlives Switchy's window would avoid that.

Turning the proxy *on* mid-session works for Codex because the side of Codex that re-reads the config picks up an added address; it is the running turn that does not move off its starting one.

The app-server protocol (`codex app-server generate-json-schema`) has no provider or address on `turn/start`; `config/batchWrite` with `reloadUserConfig` re-reads the file, which matches the partial follow above. `thread/resume` accepts `modelProvider` and `config` overrides and is untested as a way to move a bridged window; the user chose the listener instead.
