# Finding: pooling Cursor CLI and Antigravity CLI, and retiring Gemini CLI pooling

**Status:** provisional. The endpoint overrides were read from the installed binaries, not exercised. Nothing is built (task SW314); the user decides between the options below.
**Observed:** 2026-11-02, workstation, Switchy 1.0.48, `cursor-agent` 2026.10.22-27dcf5f, `agy` in `%LOCALAPPDATA%\agy\bin`.

## What Switchy can read

- **Cursor:** `DashboardService/GetCurrentPeriodUsage` gives three meters for the billing month: the included total, Auto (Cursor's own models) and API-priced models, with the cycle's end. The card's usage badge already reads it.
- **Antigravity:** `v1internal:fetchAvailableModels` gives a remaining share and a reset time per model; Switchy groups them by family (Gemini, Claude, GPT-OSS). The badge already reads it.
- Both readings come from asking the service, not from response headers. Rotation would have to poll them, so it acts minutes after a crossing, not on the request that crosses it as with Claude and Codex.

## Whether the CLIs can be routed through the proxy

- **`cursor-agent`** builds its API address from `CURSOR_API_BASE_URL` (default `https://api2.cursor.sh`); `CURSOR_API_ENDPOINT` also appears. Found by searching the bundled JavaScript of the installed version.
- **`agy`** logs "Overriding CloudCodeServerURL via CLOUD_CODE_URL environment variable", and honours `HTTPS_PROXY`. Found in the executable's strings.
- Neither override is documented, so an update may remove it. Not yet checked: that a plain `http://127.0.0.1` address is accepted; that Cursor's streaming protocol passes through a forwarding proxy; and, for `agy`, whether requests carry a per-account Google project id that would have to be rewritten, as Claude's account id is.
- **Verification path:** start each CLI with the override pointed at a local listener that logs and forwards, run one turn, and read what arrives.

## Options

1. **Pooling through the proxy.** Swap each request's token for the chosen account's, like Claude Code and Codex, with rotation before the limit and the per-account switch-away limit, mid-session. Needs the override check above and a reading of Google's and Cursor's terms first: `agy`'s sign-in is Google's own OAuth client, and Switchy would be sending that account's traffic itself.
2. **Switch at the next launch.** When the current account passes its limit, Switchy enables the next card, rewriting the CLI's login as *Enable* does now; the next `cursor-agent` or `agy` started uses it, and running sessions keep their account. Uses only what Switchy already does. For `agy` this does not hold while an older session runs: each running `agy` saves its hourly token renewal back to Credential Manager, so the account it started on returns there within the hour and the next launch uses it (README § Cursor CLI and Antigravity CLI, seen 2026-11-03 in `agy`'s own logs). Rotation by launch would have to rewrite the login after each renewal, or start each `agy` with its account's login in a store of its own.

## Retiring Gemini CLI pooling

Gemini CLI is taken over by the proxy on the workstation (one card, "Google Official"). Retiring its pooling means leaving it out of *Route through the proxy*, which hands its config back, and its proxy route and request conversion (the Gemini parts of the proxy handlers, forwarder and adapter). That code comes from cc-switch, the project Switchy is forked from, so removing it makes merges from there conflict; leaving it unrouted does not. Switching Gemini CLI cards by hand does not use the proxy.

**Revisit when:** the user chooses an option, or a CLI update changes either override.
