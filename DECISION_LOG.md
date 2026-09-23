# Decision Log

Pruned 2026-09-10 to the recordkeeping model's decision test (`C:/Projects/methodology/meta/recordkeeping_model.md` § Decision); the removed entries are in git history at the pruning commit.

## 2026-09-23 — The public repo is a cleaned copy of the private one, rebuilt by a script

- Context: the user wanted Switchy public and shareable under the name `switchy`. The working repo's history carries his email addresses, his name and the session notes.
- Decision: work continues in `registered2nd/switchy-private` (origin on the workstation and the laptop). `registered2nd/switchy` is public and holds only what `scripts/publish_public.sh` produces: a clone of main with the session logs, briefs, handoff files and old browser snapshots dropped from every commit, and his emails and name replaced. The rewrite is deterministic, so rerunning the script fast-forwards the public branch. Never push the private repo to the public one or add it as a remote there.
- Why: keeping the full record private and unchanged means no force-push, no re-sync and valid commit references in this log, while the public copy stays current with one command.
- Files: `scripts/publish_public.sh`.

## 2026-09-23 — Switchy's interface has its own look; the accent lives under Tailwind's blue-* scale

- Context: the interface still looked like cc-switch (pill app switcher, a Settings tab bar, accordion cards with colored icon tiles, glass panels, blue and green accents). the user asked for a new UI across the main window and every Settings page.
- Decision: a left sidebar (`src/components/layout/AppRail.tsx`) carries navigation: apps, Usage Statistics and Settings, and inside Settings its sections; `SettingsPage` renders the section the sidebar picks (`activeTab`). Settings content is plain `SettingsSection` blocks, collapsible only for rarely-touched configuration. Accounts are lines in one bordered list, with quota as meters (`TierMeter`). Theme tokens in `src/index.css`: graphite surfaces, brass `--primary`, `--success`, `--rail`; flat panels (`.glass`/`.glass-card` are a surface and a hairline). Type is IBM Plex Sans and IBM Plex Sans Condensed (`font-display`), bundled with `@fontsource`. The Tailwind `blue` scale is redefined as the brass accent, so the cc-switch components' hardcoded `blue-*` classes take the accent; new code uses `primary`, not `blue-*`. Selected options are a card-colored segment with a brass underline, never a solid fill; solid brass is for primary actions and what is live.
- Why: the user — Switchy should not look like cc-switch, in every tab. Remapping `blue-*` recolored 122 uses in 29 files at once instead of editing each.
- Files: `src/index.css`, `tailwind.config.cjs`, `src/main.tsx`, `src/components/layout/AppRail.tsx`, `src/components/settings/SettingsSection.tsx`, `src/components/settings/SettingsPage.tsx`, `src/components/providers/ProviderCard.tsx`, `src/components/SubscriptionQuotaFooter.tsx`, `src/components/ui/{switch,tabs,toggle-row}.tsx`.

## 2026-09-23 — Each Codex window runs on its own app-server, which Switchy signs in to the account in use, so an open window's `/status` follows a switch (experimental, on by default)

- Context: proxy mode exists so an open Codex window switches accounts, and the user checks a switch with `/status`. A Codex window keeps its signed-in account for its life: a changed `auth.json` of another account is refused (`reload_if_account_id_matches` in `codex-rs/login/src/auth/manager.rs`), so the proxy moved the requests while `/status` showed the old account. Pointing `chatgpt_base_url` at the proxy changed only the limits `/status` fetched, needed HTTPS, and made Codex skip its agent-identity token (fetched only for chatgpt.com's own addresses), which broke its built-in apps connector; it was removed. One shared app-server (`codex app-server daemon`) that windows attach to switched `/status` but ran every window's hooks with the server's own environment (`hooks/src/registry.rs` snapshots the process environment), so Orca's hooks lost the pane variables and its sidebar lost the windows; it was replaced.
- Decision: with Settings → Pool → *Switch Codex's signed-in account* on (default) and Codex routed through the proxy, each interactive `codex` window in the WSL install runs on its own app-server, started from its terminal by a shell function Switchy writes to `~/.codex/switchy/codex.sh` and sources from `~/.bashrc` (`codex app-server --listen unix://~/.codex/switchy-sessions/<pid>-<n>.sock`, then `codex --remote` on it). The function records each socket in a `.session` file and stops the server when the window exits or its shell goes away. Switchy lists those files through `\\wsl$`, connects to each server through `codex app-server proxy --sock` (a WebSocket over stdio), and on every change of the current Codex provider or its access token sends `account/login/start` with `type: chatgptAuthTokens`. The server keeps the tokens in memory (no `auth.json` write) and sends `account/chatgptAuthTokens/refresh` to this connection on a 401; Switchy answers with a forced renewal, so it is the only holder renewing the login. Non-window commands (`exec`, `login`, `mcp`, …) and options Codex refuses with `--remote` (`--no-daemon`, `--add-dir`, `--worktree`) run plain Codex, as does every window while `~/.codex/switchy-sessions/enabled` is absent (setting off or Codex not routed). Native Windows Codex is not covered: Codex's server refuses to run elevated, and everything on this machine runs elevated.
- Verified live on codex-cli 0.156.1 in WSL: an open window's `/status` changed from registered2nd (Pro, 100% left) to user-b (Pro Lite, 50% left) on a sign-in, matching `/wham/usage` for each account; the per-window server carries the terminal's `ORCA_PANE_KEY`; the window starts with the `cod` alias's options (minus its `--enable web_search=live`, which Codex 0.156.1 rejects with or without the function); closing the terminal removes the server and its files.
- Why: the user — `/status` must behave as normal Codex, with the account Codex is signed in to actually switched, and Orca must keep seeing the windows. `chatgptAuthTokens` is marked `[UNSTABLE] FOR OPENAI INTERNAL USE ONLY` in Codex's protocol schema, so it is an experimental setting and the proxy's own switching does not depend on it.
- Files: `src-tauri/src/proxy/codex_engine.rs`; `codex_shared_session` in `proxy/account_pool.rs`; the nudges in `services/provider/mod.rs`, `proxy/failover_switch.rs`, `proxy/codex_pool.rs` and `commands/settings.rs`; `src/components/proxy/AccountPoolPanel.tsx`.

## 2026-09-23 — A switch writes only what the provider owns, for every tool; the common config reaches the tool when it is saved

- Context: a provider's stored settings are filled by the switch-away backfill, so every card, relay or Official, holds an old copy of the whole settings file. Writing it back on a switch rolled the file back to that moment: Orca's hooks disappeared (07:13 on 2026-09-23), Codex's `notify`, trusted projects, plugins and MCP servers reverted, and the Windows Codex config (with Windows paths) was merged into WSL's.
- Decision: a switch, with the proxy on or off, in Windows and in the WSL mirror, and in the backups the proxy restores from, writes only the keys the provider owns and keeps the rest of the file:
  - **Claude**: the connection `env` keys (`ANTHROPIC_*`, `CLAUDE_CODE_USE_BEDROCK`/`_VERTEX`, the AWS and Google credential variables, `API_TIMEOUT_MS`, `ENABLE_TOOL_SEARCH`) and `apiKeyHelper`, for every provider. `model`, `permissions` and the rest are the user's.
  - **Codex**: an API provider owns `model_provider`, its `[model_providers]` table, `model` and `disable_response_storage`, and leaving one removes them; an Official account owns nothing in `config.toml` (its login is `auth.json`).
  - **Kimi**: `default_model` and the `[providers]` and `[models]` tables.
  - **Gemini** already merged its `settings.json` keys over the user's and writes its own `.env`; unchanged. OpenCode and OpenClaw write only the provider's own entry; unchanged.
  - **The common config** is applied when it is saved: what the old snippet set is removed from the live file and the new one merged in. A switch no longer writes it.
  - When the live file does not exist yet, the provider's settings are written as they are.
- Why: the user — the better way, for everything. What differs between providers is the endpoint, credentials and model; everything else in the file belongs to the user and the other tools that write it.
- Supersedes: the 2026-09-22 three-way merge entry's point 4 (a switch applies the difference between the two providers) and the consequence that a switch with the proxy off writes the provider's whole settings; the earlier 2026-09-23 entry limiting this to Official Claude accounts.
- Files: `merge_claude_connection_into_target`, `codex_config_after_switch`, `kimi_config_after_switch`, `apply_common_config_change` and `write_live_snapshot` in `src-tauri/src/services/provider/live.rs`; the hot-switch and backup paths in `services/proxy.rs`; `set_common_config_snippet` in `commands/config.rs`.

## 2026-09-23 — A provider picked by hand holds for 10 minutes; the saved login of Claude Code and Codex follows the pick

- Context: with Switch automatically on, Enable only put an account first in line. Its first failed request fell through to the next account and the failover switch made that one current, so a pick was undone within seconds and its error never reached the session; rotation could skip it before trying it. Codex's `auth.json` kept the login it started with, so its `/status` never showed the pick.
- Decision:
  1. **A manual pick (window or tray) holds its app for 10 minutes.** While it holds, the proxy serves only that provider: no failover, no rotation, and automatic switches (failover, recovery) do not move the app. Errors from it reach the client. When the hold ends, automatic switching resumes.
  2. **The app's saved login follows the account under the proxy.** Claude Code's moves on every switch (entry above). Codex's `auth.json` (Windows and the WSL mirror) moves to an account once that account has answered a request through the proxy, after a login Codex renewed on its own is filed with its account. Codex reads its saved login at startup straight from OpenAI, not through the proxy, and exits on a refused one before `/login` is available, so a login is written only after it has just worked. Running Codex sessions keep the login they started with, as with the proxy off.
  3. **Handing Codex's config back keeps the login Codex has** (Windows and the WSL mirror) when it is a ChatGPT login, of whichever account; the current provider's login fills in only when Codex has none. Under the proxy that login is the last account that answered, and writing the current provider's instead handed Codex a refused login on every Switchy restart.
  4. **A refused account picked by hand signs Codex out, the way `codex logout` does.** Codex's `auth.json` is deleted (the WSL mirror's first, then Windows'), so the next Codex started opens on its own sign-in screen. Codex treats any `auth.json`, even `{}`, as a ChatGPT login and exits at startup on an unusable one: verified on codex-cli 0.156.1 with a throwaway `CODEX_HOME` — no file opens the sign-in screen; `{}`, `{"OPENAI_API_KEY": null}` or a revoked login exit with `account/read failed`. It happens at the pick when the refusal is already known, and whenever a refusal is learned while the pick holds. At the pick the login is checked the way the account's card checks it (its usage is read; it is renewed only when that gets a 401), and a working login becomes Codex's saved login at once. A file holding an API key, or a new sign-in of the same account not yet filed with it, is kept; a sign-in, on either side, is filed with its card as soon as the login watcher sees it (and before each Codex request is routed), which clears that account's failure count and circuit breaker and refreshes its card. Stored logins stay with their providers, and a working account's login is written back, recreating the file, after its first answer. A running Codex session has no sign-in of its own (no `/login`), so it keeps failing until the account is signed in or the hold ends.
- Why: the user — a manual pick is an override the pool must respect, and `/status` must show it; automatic choice is for when no one has picked. Codex must behave exactly as Codex itself does for the account in use; Switchy makes that behaviour appear rather than substituting its own. A signed-out account therefore looks to Codex like a signed-out Codex.
- Files: `src-tauri/src/proxy/manual_hold.rs`; `select_providers` in `proxy/provider_router.rs`; `do_switch` in `proxy/failover_switch.rs`; `save_login_of_serving_account`, `keep_live_login`, `sign_codex_out_for` and `file_live_login` in `proxy/codex_pool.rs`; `check_picked_codex_login` in `services/provider/mod.rs`; `start_codex` in `services/credential_mirror.rs`; `forward` in `proxy/forwarder.rs`.

## 2026-09-23 — A Claude switch changes only the keys the provider owns; an Official account owns only the connection keys, superseding point 4 of the 2026-09-22 three-way merge entry

- Context: each Official Claude card stores an old copy of the user's whole `settings.json` (hooks, plugins, permissions, model, telemetry `env`), and the three copies differ. A hot switch applied the difference between the outgoing and incoming provider's settings, so switching from the account whose copy held Orca's hooks to one whose copy did not removed Orca's hooks from the live file (07:13 on 2026-09-23).
- Decision: a Claude switch under the proxy merges only the keys the provider owns. Every provider owns the connection keys in `env` (`ANTHROPIC_*`, `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`, `API_TIMEOUT_MS`); an API provider also owns `model`, `permissions` and `effortLevel`, the set the WSL mirror already uses. An Official account owns nothing beyond the connection keys. Everything else in the file stays as it is on disk.
- Why: Official accounts are one person's subscriptions, so what they store beyond the login is that person's settings at some past moment; applying it on a switch reverts the settings other tools and the user have changed since.
- With the proxy off, writing an Official account to the live file (a switch, or an edit of the current card) likewise replaces only the connection keys, in `settings.json` and in the WSL mirror; an API provider's settings are still written whole.
- Files: `claude_provider_owned` and `sync_claude_live_from_provider_while_proxy_active` in `src-tauri/src/services/proxy.rs`; `merge_claude_connection_into_target` and the Claude arm of `write_live_snapshot` in `services/provider/live.rs`.

## 2026-09-23 — Under the proxy, Claude Code's saved login follows the enabled account, superseding "the live login is not swapped" of 2026-09-21

- Context: with the proxy serving Claude, a switch changed only the account the proxy presented; Claude Code's saved login stayed on whatever account it was signed in to, so `/status` never changed and the user could not see whether a switch had worked. The reason for keeping it (swapping the saved login moves every open session at once, which had corrupted terminals) does not apply in Orca.
- Decision: every Claude switch under the proxy (Enable, rotation, failover, recovery) also runs the switch-with-proxy-off login swap: the outgoing live login is saved to its account, the new account's captured login and identity are written, and the live-owner marker names it. The proxy keeps presenting the enabled account's captured login on inference calls; Claude Code's identity calls now carry the same account.
- Why: the user — `/status` is how he checks that switching works, and two identities per session are hard to monitor.
- Files: `swap_claude_login` in `src-tauri/src/services/provider/mod.rs`, called from the hot-switch branch there and from `proxy/failover_switch.rs`.

## 2026-09-23 — Only failures about the account count against it; every account switch is recorded with its reason

- Context: every failed request counted toward the account's health and circuit breaker, including Anthropic's 400 "prompt is too long" and the 429s that hit all three Claude accounts in the same second when one oversized request went round them. Healthy accounts showed "Degraded", and five such requests take an account out of rotation. The usage view could not say which account served a request or why the pool moved.
- Decision:
  1. **An error counts against an account only when it is about the account**: a refused login (401/403), a server error (5xx), a timeout or a dropped connection. Other errors (400, 404, 413, 429, a cancelled request) still move the request to the next account but leave the health count and the breaker alone.
  2. **Every change of the account serving an app is recorded** (`account_switches`, 90 days) with a reason: picked by hand, a failed request, a usage limit (429), a refused login, rotation near the limit or after the breaker opened, or an earlier account recovering. The usage view is built around accounts: per-account requests, tokens, 429s and last use over the chosen range, and the switch history.
  3. **A refused login is shown as one**: the card says *Signed out* with how to sign in again, and a notice names the account when a request finds it refused.
- Why: an account's health label and breaker must describe the account, or oversized requests knock good accounts out of rotation. Without the reasons, a switch the pool made looks like an Enable click that did nothing.
- Files: `counts_against_provider` and `switch_reason_for` in `src-tauri/src/proxy/forwarder.rs`; `database/dao/account_switches.rs`; `needs_sign_in` in `proxy/codex_pool.rs` and `proxy/claude_pool.rs`; `src/components/usage/`.

## 2026-09-23 — Switchy keeps only switching and pooling; code is English and Chinese exists only as a translation

- Context: the fork still carried upstream cc-switch's MCP, prompt and skill managers, deep-link import, WebDAV sync, the auto-updater, the session browser, the Claude plugin and onboarding toggles, partner promotion and the config.json migration, with Chinese comments, log lines and error text throughout the code.
- Decision:
  1. **Removed**: the MCP, prompt and skill managers, deep links, WebDAV sync, the updater, the session browser, the Agents placeholder, the Claude plugin and onboarding toggles, partner promotion and the one-time migrations. Kept: every provider type and preset, universal providers, OpenCode and OpenClaw, usage scripts, coding-plan quota, the speed test and session repair.
  2. **Code, comments, logs and backend errors are English.** User-visible text goes through the locale files, and `zh.json` is a natural Chinese translation of `en.json` with the same key set. Backend errors carry English only.
  3. **Tray labels come from the locale files** (`tray.*`), read into the Rust build.
- Why: the user — anything not useful for switching and pooling goes; Chinese belongs in the Chinese version, not in the code. Removed features can come back later as Switchy's own.
- Files: `src-tauri/src/lib.rs` (command list), `src-tauri/src/tray.rs`, `src/i18n/locales/*.json`.

## 2026-09-23 — The takeover covers the WSL install: its sessions go through the proxy too

- Context: every Codex and Claude session the user runs is in WSL, and the takeover edited only the Windows files, so enabling or rotating an account under the proxy never reached them — the case the proxy was built for.
- Decision:
  1. **Taking an app over also points its mirror install at the proxy** — the Claude or Codex mirror directory a switch already keeps in step. Claude gets the same takeover fields as Windows; Codex gets `openai_base_url`, and only when it is signed in with ChatGPT on the built-in provider.
  2. **Only when WSL can reach the proxy**: a WSL mirror is taken over only with `networkingMode=mirrored` in `.wslconfig`, which makes Windows' loopback address WSL's. Pointing a NAT-networked WSL at `127.0.0.1` would cut its sessions off.
  3. **The mirror is backed up and handed back by the same three-way merge**, under backup keys of its own (`claude_mirror`, `codex_mirror`). A provider switch made during the takeover is carried into the mirror backup the way a switch writes the mirror with the proxy off, and a Codex account switch writes that account's login to the mirror when the proxy lets go, unless the mirror holds a newer login of the same account. *(Login part superseded 2026-09-23: when the proxy lets go, Codex keeps the ChatGPT login it has, on Windows and in the mirror.)*
  4. **Startup recovery covers the mirror**: a mirror still pointing at the proxy counts as a leftover takeover, and with no backup the proxy address is removed.
- Why: WSL talks to OpenAI and Anthropic directly otherwise, so neither mid-session switching nor rotation applies to the sessions that exist.
- Files: the mirror section of `src-tauri/src/services/proxy.rs` (`take_over_mirror`, `restore_mirror`, `update_mirror_backup`); `codex_mirror_config` in `services/provider/live.rs`.

## 2026-09-22 — Handing a live config back is a three-way merge against a record of what the takeover wrote

- Context: every restore (clean stop, recovery after an unclean exit, a takeover switched off) wrote the takeover backup over Claude's `settings.json` and Codex's `config.toml` whole. The backup is as old as the takeover, and a takeover lasts as long as Switchy runs, so everything other tools wrote to those files meanwhile was lost. Orca keeps its status hooks in `settings.json`; the recoveries at 22:45 and 22:58 on 2026-09-22 restored a backup taken before Orca put them back, and every Claude session started afterwards was missing from Orca's sidebar.
- Decision:
  1. **A restore is a three-way merge.** The base is what the takeover last wrote, recorded in the backup row (`written_config`); ours is the backup, or the current provider's settings when there is no backup; theirs is the file on disk. Switchy's changes (base → ours) are applied, everyone else's (base → theirs) are kept, and where both changed one value the file on disk wins. `settings.json` is merged as JSON, `config.toml` as TOML keeping the file's formatting and comments. Codex's `auth.json` keeps its login rule; Gemini's `.env` is written from the backup as before.
  2. **The record is Switchy's own content**: the takeover's write, or the provider settings a hot switch or provider edit asked for — not the merged file. A record of the merged file would make the next restore count other tools' keys as Switchy's and remove them.
  3. **Without a record** (a backup from an earlier build, or no backup), the base is the target with the keys the takeover manages as they are on disk, so only those keys change: Claude's `env` base URL, token and model-override keys; Codex's `openai_base_url` and `base_url`s. A hot switch that replaces a backup with no record first records one from the outgoing backup this way.
  4. *(Superseded 2026-09-23 for Claude: a switch changes only the keys the provider owns.)* **A hot switch or provider edit under the takeover applies only the difference between the two providers.** The base is the outgoing provider's effective settings with the takeover fields, so keys neither provider owns stay, whether they were in the file before the takeover or written during it. A Codex backup is carried across the switch the same way, keeping what it held beyond the outgoing provider's config.
- Why: putting back only the takeover's keys is not enough, because a hot switch or a common-config edit changes the backup without touching the live file (always for Codex, for common config on Claude), and those changes still have to land when the takeover ends. The record is what separates them from other tools' edits.
- Consequence: with the proxy off, a switch still writes the provider's whole settings file, so a key its settings (with the common config) lack is removed — Orca's hooks included, unless they are in the Claude common config. *(Superseded 2026-09-23 for Official accounts, which now write only the connection keys.)*
- Files: `src-tauri/src/services/live_merge.rs`; `config_to_restore`, `merge_onto_live`, `takeover_base` and the takeover writes in `services/proxy.rs`; `start_live_backup`, `save_live_backup` and `record_live_written` in `database/dao/proxy.rs`.

## 2026-09-22 — Enable stays under Switch automatically and puts the account first; a Codex login made under the proxy is filed before routing

- Context: upstream replaces a card's Enable with a queue toggle while an app switches automatically, so no account could be picked by hand and none made current. Signing in depends on the account being current, and under the proxy a switch is a hot switch with no switch-away backfill, so a `codex login` was never stored unless the account already held a login.
- Decision:
  1. **Enable is always Enable.** With Switch automatically on, enabling also adds the provider to the switching order, which lists the current provider first and the rest in sort order. The cards keep their places: moving the enabled card to the top put the other card under the pointer, and a second click switched back. Queue membership is a separate list button on the card.
  2. **Before each Codex request is routed, the live `auth.json` is filed**: a login of an account a provider holds goes to that provider under the existing newest-wins rules; a login of an account no provider holds goes to the current Official provider only when that holds no usable login.
- Why: the user — Enable should never disappear, and there was no way to sign a new account in. A merge from upstream would bring the queue-toggle button back.
- Files: `add_to_switching_order` in `services/provider/mod.rs`; `get_failover_queue` in `database/dao/failover.rs`; `file_live_login` in `proxy/codex_pool.rs`, called from `select_providers`; `src/components/providers/ProviderActions.tsx`.

## 2026-09-22 — Rotation counts the requested model's own limit window, not only the account-wide ones

- Context: live `anthropic-ratelimit-unified-*` headers show a Fable answer carries a model-scoped 7-day bucket (`7d_oi`, 83% on the account tested) that Opus and Sonnet answers do not. Counting only account-wide buckets let an account out of Fable allowance look fresh for Fable; the overall `rejected` status of a Fable-only refusal then benched it for every model; and each answer replaced all stored windows, so an Opus answer erased the Fable reading.
- Decision:
  1. **Rotation is per request model.** `select_providers` takes the request's model; an account is passed over when an account-wide window, or a window that applies to that model, is at the threshold.
  2. **Which scoped window applies to a model is learned from answers**, since Anthropic's bucket names (`7d_oi`) do not name the model and each answer carries only the buckets that apply to the model that answered. Codex names its scoped windows after the model, so there they match by name.
  3. **A scoped refusal benches the account for that model only**; account-wide windows are replaced per answer, scoped ones merged by name.
- Why: the user's case — a session on Fable must move when Fable is spent across accounts, while Opus on the same account may still be free.
- Consequence: after a restart, the first request for a model sees only the account-wide windows until an account has answered that model once. Partly supersedes point 4 of both 2026-09-21 pool entries below.
- Files: `src-tauri/src/proxy/account_pool.rs` (`MODEL_SCOPES`, `record_windows`, `is_spent`), `parse_quota_headers` / `record_quota` in `proxy/claude_pool.rs`, `select_providers` in `proxy/provider_router.rs`.

## 2026-09-22 — The fork's own account-pool controls are the Pool tab; upstream's "failover" is renamed Switch automatically

- Decision: the inherited Settings → Proxy tab is **Pool**, led by one card with the switches in dependency order (local proxy, per-app routing, per-app *Switch automatically*, rotation, keep-warm); the proxy server, switching order, rectifier and outbound proxy sit collapsed below. "Auto failover" / "failover queue" are **Switch automatically** / **switching order** in all English UI text; i18n keys keep their upstream names.
- Why: the user found the upstream framing hid the pool behind a proxy tab and the word "failover" did not say what the switch does. A merge from upstream cc-switch would otherwise bring both back.
- Files: `src/components/settings/PoolTabContent.tsx`, `src/components/proxy/PoolControls.tsx`, `src/i18n/locales/*.json`.

## 2026-09-21 — Keep-warm opens pooled accounts' session windows on a timer, off by default, reversing the rest of 2026-04-20's refusal of unattended calls

> **Point 4 amended 2026-09-22** — a ChatGPT-login Codex account is refused API models, including the default Codex test model. It is warmed with the test model only when Codex's own model cache (`~/.codex/models_cache.json`) lists it; otherwise with the model that list shows last, at its lightest effort. The list is read rather than a model named, so it follows OpenAI's retirements. Verified live: `gpt-5.5` at `low` returned 200 with the `x-codex-*` quota headers on `user@example.com`.

- Context: rotation moves the session onto the next account in the queue, but a subscription's session window (Anthropic's five hours, ChatGPT's equivalent) only opens on a real request. An account nobody has used is therefore cold when rotation reaches it: the whole window starts then, and that account is the one holding the session with its reset furthest away. It also reports no quota at all, so `is_spent` knows nothing about it until the first request has already been spent finding out. the user directed keep-warm after it was established that a request which opens the window necessarily refreshes the credential too, so one feature covers both.
- Decision:
  1. **One timer, one single-token request per account per window.** A sweep every five minutes looks at each Official Claude and ChatGPT-login Codex provider and sends `max_tokens: 1` / `"hi"` to the account whose session window is not running. An account whose window is still running is skipped, and so is one at its limit, which holds the bill near one request per account per window regardless of the interval. The interval setting (default 60 minutes) is the floor between two attempts for one account, not a send rate.
  2. **Off by default, and its own switch.** Neither the proxy nor rotation needs to be on; keep-warm works on the stored logins directly, because the accounts worth warming are exactly the ones nothing is currently routing traffic to. It runs while Switchy runs.
  3. **The same path a real request takes.** Exit check first, then the account's own login through `access_token_for` / `credentials_for` — so renewal happens as a byproduct under the existing holder rules, a rejected refresh token is not re-sent, and a renewed login is written through to the live store and the takeover backup exactly as it is for a request the user made. The answer's rate-limit headers are recorded for rotation, including a refusal's.
  4. **The model is the health-check test model** for that tool (`StreamCheckConfig`), which already defaults to the cheapest one for each. No new model setting, and no model id baked into a background feature that would go stale.
  5. **The last attempt time is persisted**, so a restart does not re-warm every account: the in-memory quota store is empty at launch, which otherwise reads as "no window running" for all of them.
- Why: the alternative to warming is discovering an account's state by spending a real request on it, which costs the user's turn rather than a token. What this buys is bounded and honest; what it costs is the thing 2026-04-20 refused, so it is off by default and the README states the limit that cannot be engineered away.
- Consequence:
  - **2026-04-20's refusal of the periodic background loop (its BACKLOG #6) no longer stands.** Renewal now happens on a timer with nobody present, not only when the proxy is about to present a login. The policy question it raised is unchanged and unresolved: this makes authenticated requests with subscription credentials from something that is not the CLI itself. It was put to the user, who directed it built with the switch off.
  - **Keep-warm cannot keep an account signed in.** The refresh token's own deadline is anchored to the last browser sign-in and renewing does not move it, so an account still needs `claude /login` or `codex login` when that passes. Stated in the README rather than left to be discovered.
  - Verified live on 2026-09-21 for Claude: the request this builds returns 200 for 8 input tokens and 1 output token, and its answer carries the `5h` and `7d` unified windows the pool reads, with the 5-hour reset in the future — which is what makes the next sweep skip that account.
  - **Not verified for Codex**, because neither account can serve a request: `registered2nd@gmail.com`'s login is answered `401 token_revoked`, and `user@example.com` is at its 7-day limit until 2026-09-25T23:49Z (both read from upstream's own usage endpoint). Its request is built in the shape Switchy's shipped Codex health check already uses, plus the account id header the ChatGPT backend needs.
- Files: `src-tauri/src/proxy/keep_warm.rs`; `keep_warm_enabled` / `keep_warm_interval_minutes` and `session_window_reset` in `proxy/account_pool.rs`; the startup call in `lib.rs`; `src/components/proxy/AccountPoolPanel.tsx`.

## 2026-09-21 — Official Claude accounts are served through the proxy behind a toggle that is off by default, partly superseding 2026-04-20

> **Point 4 partly superseded 2026-09-22** — a window scoped to the requested model now counts too; see "Rotation counts the requested model's own limit window".

> **Amended the same day (1.0.13)** — the separate toggle was removed at the user's direction. Taking Claude over with the per-app Local Proxy switch is the opt-in; an Official account is then served with its captured login, and a takeover is refused only when that login has not been captured. The toggle had left Local Proxy for Claude broken for an Official account unless a second, initially greyed-out switch was set first.

- Context: Claude Code re-reads its credentials per request, so the file swap already moves an open session; what it cannot do is rotate accounts on quota without a global swap that hits every session at once and trips the open terminal-corruption finding. the user asked for the TeamClaude functions on the Claude side as well, knowing the 2026-04-20 entry had ruled out Switchy renewing Pro/Max tokens.
- Decision:
  1. **Same design as Codex, one more toggle.** With *Serve Official Claude accounts through the proxy* on, takeover sets only `ANTHROPIC_BASE_URL` (token keys are removed rather than replaced by the placeholder) so Claude Code stays in subscription mode; the proxy presents the selected Official provider's captured login, adds the `oauth-2025-04-20` beta, and patches `metadata.user_id`'s `account_uuid` to the presented account. With the toggle off, nothing changes: an Official Claude provider under takeover stays unserved, as before.
  2. **The proxy is a holder of captured logins under the 2026-08-16 rules.** Ownership of the live login is read from the recorded marker, never inferred; a newer live login of the owned account is taken before a stored one is used; a login the proxy renews is written to the snapshot and, when the marker names that account, to the live store's `claudeAiOauth` block with the marker's expiry moved along. Renewal uses Claude Code's client id and the identity its own refresh call carries, because Anthropic's edge refuses the token endpoint to anything else.
  3. **Identity-plane calls keep the client's login.** `/api/oauth/*` and `/v1/code/*` are relayed with the login Claude Code sent; only `/v1/messages*` gets the pooled login. Presenting a pooled token on the profile call makes Claude Code adopt the other account's identity (observed by TeamClaude on a live fleet).
  4. **Quota comes from `anthropic-ratelimit-unified-*`** on every answer; only the account-wide buckets spend an account, and `rejected` passes it over until the named reset. The exit check asks `api.anthropic.com/cdn-cgi/trace` and fails closed like Codex's.
- Why: The mechanism is the one already built for Codex, so the Claude side is the login-handling module plus the takeover change. The 2026-04-20 reasons were policy and Cloudflare enforcement; the user-agent Claude Code's own refresh sends passes the edge (verified by TeamClaude's use), and the policy question was put to the user with the current wording of Anthropic's page, who chose to build it with the toggle off by default.
- Consequence:
  - The 2026-04-20 abandonment stands for the captured-card quota path and for any renewal outside this toggle. Under this toggle alone a captured login is renewed only when the proxy is about to present it; the periodic loop of its BACKLOG #6 arrived later the same day as keep-warm, behind a switch of its own (see the entry above).
  - Under rotation the live login is not swapped, so the session's own identity calls run as the account Claude Code is signed into while inference runs as the pooled one. *(Superseded 2026-09-23: the saved login now follows the enabled account.)*
  - Verified live on 2026-09-21: one `claude -p` through a test proxy serving a copy of the current account (refresh token blanked) returned its reply, and the proxy recorded the 5-hour and 7-day windows from the response.
- Files: `src-tauri/src/proxy/claude_pool.rs`; `account_pool.rs` (settings, quota store, exit check shared with Codex); the `ClaudeOAuth` arms of `proxy/providers/claude.rs`, `proxy/forwarder.rs` and `handle_claude_passthrough` in `proxy/handlers.rs`; `apply_claude_takeover_fields` in `services/proxy.rs`.

## 2026-09-21 — Codex accounts switch in an open session through the proxy, which holds ChatGPT logins under the existing newest-wins rules

> **Point 4 partly superseded 2026-09-22** — a window scoped to the requested model now counts too; see "Rotation counts the requested model's own limit window".

- Context: Codex reads `auth.json` once at start and reloads it only when the account id on disk matches the one it started as (`reload_if_account_id_matches` in its auth manager), so writing another account's login cannot move an open session. Claude Code re-reads its credentials per request, which is why the file swap is enough there.
- Decision:
  1. **The proxy is the switch for an open Codex session.** A provider whose `auth` is a ChatGPT login with no API key and no endpoint of its own is served from `chatgpt.com/backend-api/codex` with that login's bearer token and `chatgpt-account-id`; the login Codex sent is dropped.
  2. **Takeover of a ChatGPT-mode install sets `openai_base_url` and leaves `auth.json` alone.** The API-key placeholder would flip Codex into API-key mode. A custom `model_provider` would work too, but Codex's resume picker lists only the sessions of the current provider id, so every earlier session would drop out of `codex resume`. The cost of staying on the built-in provider is handled in the proxy: request bodies arrive zstd-compressed, and Codex tries a WebSocket first, which is answered 426 so it falls back to HTTP at once.
  3. **The proxy is one more holder of each login, under the 2026-09-12 rules, not a separate sign-in.** Before using a stored login it takes a newer one for the same account from `auth.json`; after renewing one it writes the result to the provider, to `auth.json` when that holds the same account, and to the takeover backup; restoring the backup never overwrites a newer live login of the same account. A rejected refresh token is never re-sent, and a 401 within a minute of a renewal does not trigger another.
  4. **Rotation is the failover queue ordered by quota**, read from the `x-codex-*` headers of every response and from usage-limit refusals. Only account-wide windows count; spent accounts go to the back rather than out, so upstream's own message still reaches the user when all are spent.
  5. **The exit is checked before a ChatGPT login is used, and the check fails closed.** `chatgpt.com/cdn-cgi/trace` is asked over the request's own route; a blocked country (default `CN`) or no answer holds the request up to 30 s and then refuses it.
- Why: TeamClaude (`KarpelesLab/teamclaude`) does the same job with its own browser sign-in per account, because it has nothing that reconciles two holders of one login — its code records 287 consecutive rejected renewals from that race. Switchy already reconciles holders (2026-08-16, 2026-09-12), so sharing the login costs nothing and keeps one sign-in per account. The exit check exists because a login presented from the wrong region is answered 403, which Codex reads as a dead session, after the account has already been seen there.
- Consequence:
  - Claude gets the same path behind its own toggle, off by default — see the entry below.
  - Renewing a ChatGPT login from Switchy uses the Codex CLI's OAuth client id against `auth.openai.com`. OpenAI's terms on this were not reviewed.
  - ~~A Codex install in WSL is not routed through the Windows proxy.~~ Superseded 2026-09-23: the WSL install is taken over too; see "The takeover covers the WSL install".
- Files: `src-tauri/src/proxy/codex_pool.rs`; the ChatGPT arms of `proxy/providers/codex.rs`, `proxy/forwarder.rs` and `proxy/handlers.rs`; `apply_codex_takeover_fields` in `services/proxy.rs`; `src/components/proxy/AccountPoolPanel.tsx`.

## 2026-09-12 — Kimi Code is an exclusive-mode app whose provider is the whole `config.toml` plus its login file

- Context: Kimi Code (`~/.kimi-code/`) keeps several providers side by side in one `config.toml` and selects one with `default_model` — structurally closer to OpenCode's additive file than to Codex. Its account login lives separately, in `credentials/kimi-code.json`.
- Decision: Treat Kimi like Codex, not like OpenCode. A Kimi provider stores `{ "config": <the whole config.toml text>, "credentials": <kimi-code.json, or null> }`. Switching writes both files together (the credentials are rolled back if the config write fails; `null` removes the login file), and the ordinary switch-away backfill reads both back into the provider being left.
- Why:
  1. **The login is the point.** In additive mode every provider would share one login file, so switching could never change which Kimi account is signed in — the capability Switchy exists to provide for Claude and Codex.
  2. **It is Codex's shape** (`{ auth, config }`), so backfill, TOML common config, the form components and deep links all reuse existing paths; Kimi needed no switching logic of its own.
  3. **`default_model` alone cannot switch.** Each provider brings its own model tables, thinking settings and service blocks; storing the whole file keeps those with the provider that uses them.
- Consequence:
  - Kimi's MCP servers live in `mcp.json`, outside the provider, and are synced by the MCP service — switching never drops them. Kimi common config is everything except `default_model`, `[providers.*]` and `[models.*]`.
  - Kimi publishes no usage figures, so Kimi cards carry no usage badges.
  - The Kimi switch-away backfill has no account guard: a `kimi login` as a different account while a provider is current is filed under that provider. The 2026-09-12 Codex rules below are not applied to Kimi.
- Files: `src-tauri/src/kimi_config.rs`; the Kimi arms of `services/provider/live.rs` and `services/provider/mod.rs`; `brief_archive/kimi_frontend_checklist.md` (the frontend contract).

## 2026-09-12 — Codex login ownership is read from the login itself; the 2026-08-16 switch-away rules apply to Codex through it

- Context: The 2026-08-16 entry made Claude's live-credential ownership *recorded, not inferred*, because Claude's credentials blob carries no account identifier. Codex's `auth.json` does: its `tokens` block holds `account_id` and an id token naming the account, and the file records `last_refresh`.
- Decision: For Codex, the switch-away backfill and the WSL reconciler identify the account from the login's own claims (`tokens.account_id`, then the id token's `chatgpt_account_id`, then its email) and order freshness by `last_refresh`. There is no ownership marker. The rules are Claude's: a blank or logged-out live login never replaces a stored one; an older login never replaces a newer one; a different account's login is refused for the provider being left and moved to the provider that already holds that account, when that provider's copy is older.
- Why: A recorded marker answers "whose tokens are these?" when the tokens cannot. Codex's tokens can, and reading the answer from them stays right even for a login Switchy never wrote (a manual `codex login`), which a marker cannot cover. Keeping the rules identical to Claude's leaves one model across both tools.
- Consequence:
  - An `auth.json` with no `tokens` (an API-key provider, or a fresh Official preset) has nothing to protect and is always backfilled. The reconciler never touches an install that is on an API key.
  - The Codex mirror directory is auto-detected from the default WSL distro like Claude's, and is suppressed under `SWITCHY_TEST_HOME` for the 2026-08-16 reason.
- Files: `src-tauri/src/services/codex_account.rs`; the Codex backfill in `services/provider/mod.rs`; `reconcile_codex` in `services/credential_mirror.rs`; `write_codex_mirror` in `services/provider/live.rs`.

## 2026-08-16 — One resolver owns `.claude.json`; account state travels as a bounded allowlist; live-credential ownership is recorded, not inferred

- Decision: Four related rules for the Claude account swap, shipped as **1.0.8**.
  1. **The live `.claude.json` is resolved in exactly one place** (`config::get_claude_config_json_path`), and it is the **sibling** of the Claude config directory (`~/.claude` → `~/.claude.json`), never a file inside it. `get_claude_mcp_path` delegates to the same resolver.
  2. **Account state is a bounded allowlist** of root-level keys captured and restored alongside `oauthAccount` (`merge::ACCOUNT_STATE_KEYS`), applied with **remove-on-absent** semantics. `machineID` and `userID` are excluded as install identity.
  3. **Whichever provider owns the live credentials is recorded when they are written** (`~/.switchy/accounts/live_owner.json`), and the switch-away sync attributes them by that record — additionally gated on health, freshness, and the live identity still matching the record.
  4. **The auto-detected WSL mirror directory is suppressed when `SWITCHY_TEST_HOME` is set**, so tests cannot reach it.
- Why:
  1. **Two writers in one binary disagreed about which file is Claude's config.** `claude_account::paths` selected `~/.claude/.claude.json` whenever that file existed, while the MCP writer used `~/.claude.json`. On a machine where both exist, every account swap wrote the identity into a file Claude Code never reads: the credentials swapped correctly and requests really did run on the new account, while `/status` displayed the previous one indefinitely. An existence check cannot decide this — the answer is a rule, not a probe. Upstream `cc-switch` encodes the same sibling rule (`config.rs`), so this converges rather than diverges.
  2. **`oauthAccount` is no longer the whole account.** Claude Code records usage utilization, subscription availability, extra-usage reason and model-access caches as *siblings* of it. Restoring identity alone left the account line correct and everything around it describing the account just left. Remove-on-absent is required for the same reason: an account that never had a key must not inherit the previous one's value.
  3. **The credentials blob carries no account identifier**, so attribution previously came from reading the identity file and assuming the two agreed. They can disagree — a manual `claude /login`, or (as in 1) an app that had been writing the identity to the wrong path. A wrong guess files one account's tokens under another account's snapshot, destroying a working login. Recording the pairing at the moment it is created removes the guess; the health/freshness gates are the reconciler's rules (2026-06-13) extended to the snapshot store, which sat outside them.
  4. **The mirror default is a machine-global side-channel `SWITCHY_TEST_HOME` cannot redirect.** On any developer machine with WSL, `build_default_claude_mirror_dir()` resolves a real path, so "no mirror configured" tests silently became mirror tests — and wrote into the real WSL home. Same reasoning as the macOS Keychain bypass (2026-07-05).
- Consequence:
  - Snapshots captured before 1.0.8 have no `account_state.json`; the swap restores identity alone and logs that a re-capture is needed, rather than failing.
  - The first switch after upgrading finds no ownership marker and skips the switch-away sync by design, recording one instead. This is what makes the upgrade safe on a machine whose two config files currently disagree.
  - The ad-hoc "also write the home-root `.claude.json`" block in `claude_account::mod` is deleted; the corrected mirror path makes it redundant.
  - Switchy no longer maintains a `.claude.json` living inside a config directory. An install that genuinely sets `CLAUDE_CONFIG_DIR` keeps its config there and is not swapped unless configured as a mirror — see `LEARNINGS.md`.

## 2026-07-05 — macOS Keychain capture/swap implemented (the deferred v2), superseding the 2026-04-16 macOS-out-of-scope call

- Decision: Implement Claude-account **capture and swap on macOS** by reading/writing Claude Code's login-Keychain item (`Claude Code-credentials`) instead of the `~/.claude/.credentials.json` file used on Windows/Linux. Closes the macOS gap the 2026-04-16 entry deferred to "v2."
- Why:
  1. Capture hard-failed on macOS with "No Claude Code login found" even when logged in — Claude Code on macOS stores its OAuth blob in the Keychain, not a file; swap wrote a file Claude Code never reads, so switching silently didn't take. The feature was effectively unusable on the platform.
  2. The read half already existed in-tree: `services::subscription` reads the same `Claude Code-credentials` item via the `security` CLI. Low incremental cost, proven pattern.
- Approach:
  - New macOS-only `services/claude_account/keychain.rs` shells out to `security` (no new dependency, consistent with `subscription.rs`). Capture, swap-restore, and switch-away sync route through platform-abstracted `read_live_credentials` / `write_live_credentials` helpers in `mod.rs`.
  - Writes update the existing Keychain item **in place** (`add-generic-password -U` against the item's own `acct`) so a re-capture or account swap overwrites the current login rather than duplicating it.
  - Tests bypass the Keychain via `SWITCHY_TEST_HOME` (the Keychain is a global side-channel the file redirect can't sandbox) so they stay hermetic.
- Consequence:
  - Windows/Linux behavior unchanged (file path preserved).
  - Shipped as **1.0.7** (merged to `main`); macOS CI build compiled clean; DMG released to `switchy-dist`. **Not yet runtime-verified on a real Mac** — see `NEXT_SESSION.md`.
  - Known macOS UX caveat: reading the Keychain item may raise a one-time "security wants to access…" prompt (choose Always Allow), since Switchy isn't signed under Claude Code's identity.

## 2026-06-13 — Credential mirror: bidirectional + health-aware + account-guarded, superseding one-way

- Decision: Replace 1.0.5's one-way (live → mirror) credential-mirror watcher with a **bidirectional, health-aware, account-guarded reconciler** (1.0.6). It propagates the freshest *valid* `claudeAiOauth` bundle in whichever direction is stale, but only between sides that are the **same account** (matched by `oauthAccount` UUID); it never propagates a dead/blanked bundle, and it syncs only the `claudeAiOauth` block (preserving each machine's `mcpOAuth`).
- Why:
  1. **One-way re-armed the race instead of ending it.** The 2026-05-06 design assumed Win is the sole refresher and WSL a passive downstream consumer. In practice WSL runs its own (multiple) long-lived Claude Code processes that refresh on their own ~8h schedule. The one-way watcher continuously fed WSL a fresh token, keeping it eligible to win the refresh race; when WSL won, Win was left on a dead token (401), and the one-way copy then propagated Win's *blanked* file onto WSL too — killing both. Turning Switchy off (no re-arming → WSL falls out of the chain) is what made the 401s stop, which is the proof the watcher was the driver.
  2. **A running Claude Code session re-reads `.credentials.json` per request** — demonstrated by Switchy's own mid-session account-switch working immediately. So writing the winner's fresh bundle back to the loser's file auto-recovers it; the file layer *is* the right place to fix this, provided the sync is bidirectional. (See `LEARNINGS.md`.)
  3. **Not naive last-writer-wins (the BACKLOG #10 sketch).** mtime-newest would copy a blanked failed-refresh file (it's the newest write) over a good one. Freshness must be judged by `expiresAt` among *valid* bundles only, and direction gated by same-account — otherwise a mid-switch transient or a deliberate per-machine login would get clobbered/reverted.
- Consequence:
  - 1.0.6 ships the reconciler (`services/credential_mirror.rs`, full rewrite, 13 unit tests). `start()` wiring in `lib.rs` unchanged.
  - The swap-time `.credentials.json` mirror in `mod.rs` is retained (first-swap population); BACKLOG #12's "keep it" call stands.
  - BACKLOG #10 is closed (built, beyond the naive last-writer-wins it described).
  - Residual: the rare sub-second simultaneous-refresh tie still briefly fails the loser, which then auto-heals on its next request (~poll interval). True zero-blip needs a single-refresher auth-broker (out of scope; Switchy's proxy layer could host it).

## 2026-05-28 — Fork ships its own unsigned macOS DMG; tracks current installers in git; retires upstream cc-switch versioning

- Decision: (1) Build the fork's macOS DMG via a new on-demand **unsigned** workflow (`build-macos.yml`, hdiutil-packaged, Apple Silicon only), separate from the inherited `release.yml`. (2) **Track the current version's installers in git** (`installers/` with a Vtype-style re-include `.gitignore`), reversing the 2026-04-19 don't-commit-installers decision. (3) Treat upstream cc-switch's `3.13.0` versioning as **retired** — the fork owns its 1.0.x line.
- Why:
  1. **The inherited release.yml can't run on this fork.** `release.yml` + the `v3.x` tags came from upstream cc-switch and require Apple Developer signing/notarization secrets the fork doesn't possess. Rather than buy a $99/yr Developer ID for a personal fork, an unsigned hdiutil DMG (Vtype's pattern) gives a working macOS artifact for zero cost. Gatekeeper is cleared per-install with `xattr -dr com.apple.quarantine /Applications/Switchy.app`.
  2. **Apple Silicon only, not universal** — the user's machines are Apple Silicon; the x86_64 slice doubled runner compile time for no benefit.
  3. **Tracking installers reverses 2026-04-19 deliberately.** That decision kept installers out of git to avoid binary bloat. The countervailing pull now: the user wants the current cross-platform installers gathered and visible in the repo like Vtype does. Mitigated by the re-include pattern tracking ONLY the current version — old versions are pruned by postbuild and never committed, so bloat stays bounded to ~3 files.
  4. **Retiring 3.13.0 removes version confusion.** 3.13.0 (and the stray 1.1.0) outrank 1.0.5 numerically but are older and not the fork's line. All three version-of-record files agree on 1.0.x; the fork's history starts fresh from 1.0.0.
- Consequence:
  - `build-macos.yml` is the fork's macOS build path; trigger via Actions → Run workflow. Re-runs are fast once the cargo cache is warm.
  - `release.yml` left intact but **dormant** — not deleted, in case upstream-style signed releases are wanted later (would need a Developer ID + secrets).
  - `scripts/postbuild.mjs` is now the single promote-and-prune mechanism for Windows installers; `installers/` is the canonical distributable location.
  - macOS support is verified by **source inspection** (cross-platform `get_home_dir`, cfg-gated WSL code) — **not** by launching the build on a real Mac. First actual Mac launch is the outstanding verification.

## 2026-05-06 — Credential mirror watcher: ship the file watcher, one-way live → mirror

> **Superseded by 2026-06-13 (bidirectional + health-aware reconciler)** — one-way kept WSL armed to win the refresh race and then propagated the loser's blanked file, killing both sides. Replaced with a bidirectional, account-guarded, health-aware reconcile. The diagnosis of the rotation race itself (why sharing one credential across the WSL boundary needs sustained sync) remains correct.

- Decision: Ship a `notify`-backed FileSystemWatcher on `~/.claude/.credentials.json` that copies to `mirror_dir/.credentials.json` on every change. **One-way live → mirror only**, not bidirectional. Retain the existing swap-time `.credentials.json` mirror in `services/claude_account/mod.rs` for the first-swap-into-an-account case (before the watcher has anything to fire on).
- Why:
  1. **The 2026-05-04 entry's conclusion was wrong, and this session's forensic data corrects it.** That entry decided the post-switch refresh race wasn't supported by observation, based on a 20-minute manual-cp re-test where both sides held identical sha256. Today's WSL 401 traced to exactly the rotation race: Win refreshed `R0 → R1` over the evening, WSL was left holding R0, WSL's later refresh attempt got rejected, Claude Code blanked WSL's refresh_token field (length 0). The 2026-05-04 test was within access-token validity — neither side *needed* to refresh in 20 minutes, so the race couldn't manifest. SESSION_LOG today has the full timeline.
  2. **OAuth refresh tokens are server-rotated single-use.** Once two filesystem stores diverge from a shared starting credential, whichever side refreshes first invalidates the other side's refresh token. Sharing `.credentials.json` across the WSL boundary is structurally doomed without sustained sync — exactly what the watcher provides.
  3. **One-way (not bidirectional) is the right shape today.** Win is the active refresher in normal use (the user runs Claude Code there continuously); WSL is the consumer. One-way live → mirror keeps WSL strictly downstream so its Claude Code never has to attempt a refresh on its own. Bidirectional last-writer-wins works in theory but adds a way for WSL's spurious refresh attempt to overwrite Win's good state. Defer until the Win-side failure mode actually bites (BACKLOG #10).
  4. **The swap-time `.credentials.json` mirror is still needed for first-swap.** The watcher only fires on changes after the live file already exists. The first time a user switches into an account, the swap-time mirror is what populates the WSL side. Removing it would force a one-app-restart wait for the watcher's startup one-shot to copy.
- Consequence:
  - 1.0.5 ships the watcher. `services::credential_mirror::start()` called from `lib.rs` setup hook on every app launch; logs each mirror under `[credential_mirror]` to `switchy.log`.
  - The 2026-05-04 decision's "do not build a watcher" guidance is **superseded** by this entry. The diagnostic logging that 2026-05-04 added remains useful for first-swap-failure diagnosis and is retained.
  - BACKLOG #9 (verify mirror via diagnostic logging) is closed by this session — the mirror IS firing; the bug was downstream rotation drift, not the swap-time write.
  - New BACKLOG items added: bidirectional last-writer-wins (#10), late-appearing watch dir (#11), and a deferred reconsideration of whether the swap-time `.credentials.json` mirror should be dropped now that the watcher exists (#12 — current call: keep it).

## 2026-05-03 — Patch-bump (1.0.3) for the WSL mirror fixes, not minor-bump (1.1.0)

- Decision: The 2026-05-02 → 2026-05-03 ship is **1.0.3**, not 1.1.0. Treat the unreleased committed fixes (`08f19e2c` switch-away sync, plus the four hover/overlay commits `4c9256bf`/`d2f365fc`/`48b10153`/`2965fd84`) as the logical 1.0.1 and 1.0.2 even though they were never tagged or released, and bump to 1.0.3 for everything that landed today.
- Why:
  1. **Nothing here is genuinely new behavior.** WSL mirror auto-detection, home-root `.claude.json` mirror, quota cache invalidation on switch, refresh pill clickability — every change is making something that was supposed to work in 1.0.0 actually work. None of them give the user a capability the 1.0.0 release notes didn't already advertise.
  2. **Initial bump to 1.1.0 was reflexive.** I framed "auto-detection without configuration" as a feature; the user pushed back that it's just removing a configuration step that was supposed to be optional anyway. Same for home-root mirror — making the existing mirror feature reach the file Claude Code actually reads.
  3. **The committed-but-unreleased fixes between 1.0.0 and today represent real semver patch increments.** Six commits since `605e0412`. Pretending they're all part of "1.0.0" undercounts; pretending they're each a release overcount. Treating them as logical 1.0.1 / 1.0.2 lets 1.0.3 actually number the third patch since 1.0.0 instead of compressing six fixes into one bump.
- Consequence:
  - 1.0.1 and 1.0.2 are never tagged, never released, never carry CHANGELOG entries — they only exist as logical reference points to make 1.0.3 numerically honest.
  - Future bug-fix-only ships continue patch-bumping (1.0.4, 1.0.5, ...). Minor bump is reserved for genuinely new user-facing capability — first time a feature does something the prior release explicitly couldn't.
  - All three version-of-record files (`package.json`, `tauri.conf.json`, `Cargo.toml`) bumped together; CHANGELOG entry header matches.

## 2026-04-20 — Abandon snapshot OAuth refresh (BACKLOG #5 and #6)

> **Partly superseded 2026-09-21, twice** — a captured login is renewed by the proxy when it is about to present it, and keep-warm renews one on a timer with nobody present (BACKLOG #6's shape), each behind a switch that is off by default. The policy reasoning below is untouched by either; nothing else here has been overtaken.

- Decision: Do not build `snapshot_oauth_refresh` (BACKLOG #5, the on-demand token refresh for captured-but-not-current accounts) or the periodic background refresh loop (BACKLOG #6). Captured-card quota pills showing "Session expired" past the access-token ceiling is the correct user-facing behavior.
- Why — two independent reasons, either sufficient on its own:
  1. **Anthropic's April 2026 "Authentication and credential use" policy explicitly prohibits it.** Verbatim: *"OAuth authentication used with Free, Pro, and Max plans is intended exclusively for Claude Code and Claude.ai. Using OAuth tokens obtained through Claude Free, Pro, or Max accounts in any other product, tool, or service is not permitted and constitutes a violation of the Consumer Terms of Service."* BACKLOG #5, even in its carefully-scoped user-initiated-only variant (5b), uses a Pro/Max refresh_token from Switchy — not from Claude Code — and the policy language names this exact case.
  2. **Cloudflare WAF actively blocks the endpoint for non-browser clients** with the Claude Code Console client_id (`9d1c250a-e61b-44d9-88ed-5944d1962f5e`). This isn't a hypothetical fragility — it's deployed enforcement. Tracked in [anthropics/claude-code#47754](https://github.com/anthropics/claude-code/issues/47754); the same block broke Hermes PKCE refresh ([NousResearch/hermes-agent#6347](https://github.com/NousResearch/hermes-agent/issues/6347)). Even if the policy changes, the engineering path is a moving target.
- Consequence:
  - `specs/snapshot_oauth_refresh/requirements.md` is retained as a historical artifact with an abandonment note at the top. Design and tasks stages are not written.
  - The 2026-04-19 entry below referenced #5 and #6 as "future improvements that extend the window." Those paragraphs are **superseded by this entry** — the window cannot be extended within ToS, full stop. The 8h access-token / ~24h refresh-token ceiling is the feature ceiling, not a lower bound to work up from.
  - If a future session considers similar "help Switchy do more with captured credentials" features, the first check is: does it make any authenticated request using Pro/Max OAuth credentials from a non-Claude-Code-non-Claude.ai client? If yes, it's ToS-prohibited, regardless of how thin or user-initiated the path is.
  - The existing captured-snapshot **quota read** path (which uses the captured `access_token`, not `refresh_token`, against Anthropic's usage endpoint) is a milder version of the same question but is not affected by this decision — it uses tokens while they're still valid, does not call the OAuth refresh endpoint, and is the feature-parity story for BACKLOG #5 of the 2026-04-19 multi-account plan which was retired when switch-away sync shipped. Leave it alone; flag if/when future policy updates extend the prohibition to usage-endpoint reads.

## 2026-04-17 - De-fork Switchy: supersede upstream namespace decision

- Decision:
  - Switchy is its own project, not a fork. The `de-fork` spec
    (`specs/de-fork/`) renamed upstream `cc-switch` / `CC Switch` /
    `com.ccswitch.desktop` identifiers, the repository remote, and the app
    data directory (`~/.cc-switch/` → `~/.switchy/`, `cc-switch.db` →
    `switchy.db`). A one-shot migration shim
    (`src-tauri/src/migrate_paths.rs`) renames existing installs in place
    on startup.
- Why:
  - Fork status was holding back the project's identity and made future
    divergence harder. Native multi-account switching (active spec) would
    have deepened the branching debt.
  - The 2026-04-07 decision below (keep upstream namespace for continuity)
    is now obsolete; the migration shim supplies the continuity by
    renaming the directory rather than leaving it under an upstream name.
- Consequence:
  - The old `farion1231/cc-switch` remote is preserved as `upstream`
    for cherry-picking; `origin` now points at `registered2nd/switchy`.
  - Users upgrading from the pre-rename build have their data auto-migrated
    on first launch. The shim is idempotent; ship for one release then
    remove.
  - Historical entries (SESSION_LOG, CHANGELOG entries describing shipped
    versions, the 2026-04-07 entry below) are preserved verbatim and
    intentionally still mention the upstream name — they describe past
    state.

## 2026-04-07 - Keep existing app data namespace for continuity

*(Superseded 2026-04-17 by the de-fork decision above. Kept verbatim as history.)*


- Decision:
  - Keep the existing internal storage/config namespace (`~/.cc-switch`, related storage keys, and inherited profiles) for now.
- Why:
  - The user explicitly wanted to preserve the current profiles/state rather than isolate the fork immediately.
  - Renaming the app identity is sufficient for installation/branding separation, while keeping data continuity avoids accidental loss of existing provider state.
- Consequence:
  - `Switchy` still reads the existing `CC Switch` app data/state.
  - Uninstalling the old app does not remove those profiles, which is acceptable and currently desired.

## 2026-04-07 - Mirror WSL Claude by provider-field sync, not full-file sync

- Decision:
  - The WSL mirror target should only receive provider-related Claude fields, not a full-file overwrite.
- Why:
  - Full mirroring copied Windows-only hooks/status-line commands into WSL and broke WSL runtime behavior.
  - The practical goal is shared provider switching across Windows and WSL without destroying machine-specific local wiring.
- Consequence:
  - Mirror target preserves local hooks/status-line/plugins and only updates provider-related fields.
  - If the mirror file cannot be parsed, mirror sync is skipped rather than falling back to a destructive overwrite.

## 2026-04-10 - Absorb cc-account-switcher capability into Switchy as native feature

- Decision:
  - Rather than adopting `ming86/cc-account-switcher` as a separate bash tool, port its credential-swap capability into Switchy as a first-class feature, exposed through the existing provider UI.
  - Concretely: when an "Official" provider is added, Switchy captures a snapshot of the active Claude Code OAuth state. When that provider is enabled, Switchy restores that snapshot to `~/.claude/` (and mirrors to the WSL `.claude/` target if configured). This allows two or more "Official" providers to represent two distinct Anthropic accounts, switchable like any other provider.

- Why:
  - Stock cc-switch (and current Switchy) only manipulates env vars / `settings.json` for "Official" providers — it cannot distinguish between two Anthropic accounts because the OAuth credentials live in `~/.claude/.credentials.json` and `~/.claude/.claude.json` (`oauthAccount` field), neither of which Switchy currently touches.
  - `ming86/cc-account-switcher` already proved the swap pattern works (back up + restore those exact files), but it is a bash script targeting macOS/Linux/WSL only and lives outside Switchy's UI/state model. Running it in parallel would split state across two tools.
  - Switchy already owns:
    - the provider switching UI and storage
    - the `~/.claude/` write path (Windows)
    - the WSL `~/.claude/` mirror write path (this fork's addition)
    so adding credential-swap is incremental, not architectural.
  - This pushes Switchy further from upstream — the user explicitly wants Switchy to "have its own life" and the WSL mirror was the first such divergence; multi-account credential swap is the second. Future divergence from upstream is acceptable.
  - The user is on Pro plan and unlocking 1M context requires either Max upgrade or enabling extra-usage billing on a Pro account. Managing two separate Anthropic accounts cleanly (each with its own subscription level / billing posture) is the underlying need that motivates this feature now rather than later.

- Consequence:
  - Switchy will need a per-provider credential cache under its existing data dir (current namespace: `~/.cc-switch/` — see prior decision to keep the upstream namespace for continuity).
  - The Rust services layer in `src-tauri/services/provider/` will gain credential snapshot/restore alongside the existing live config writers. Both Windows and WSL `.claude/` targets must be handled symmetrically with the existing mirror logic.
  - The "Official" provider concept in the UI grows a "captured account" notion (display email/UUID from `oauthAccount`) so users can tell two Official entries apart.
  - Spec-driven workflow applies: this is non-trivial, so the next session begins at `requirements.md` per `spec-quality-guide.md`. Do not skip stages.
  - Fork polish (renaming residue, end-to-end installer test, namespace isolation decision) is now backlog, not active. It is preserved in `SESSION_LOG.md` 2026-04-10 entry and should be picked up after the multi-account feature lands or when it becomes blocking.

## 2026-04-16 - Plaintext snapshots + macOS deferred to v2 (scope-shaping calls from the spec)

> **macOS half superseded by 2026-07-05** — Keychain capture/swap is now implemented (1.0.7). The plaintext-snapshot decision below still stands.

- Decisions (scope-shaping, made during the requirements/design loop for official multi-account):
  - v1 stores per-provider OAuth snapshots plaintext under `~/.cc-switch/accounts/{provider_id}/`, with `0o600` on Unix and default NTFS user-profile ACL on Windows. No encryption.
  - v1 handles Windows + WSL only. macOS Keychain capture/restore is explicitly out of scope — the Rust code path on macOS returns a clear `AppError` rather than silently succeeding with wrong state.
- Why (documented once here so future sessions don't re-derive):
  - Claude Code's own `.credentials.json` is already plaintext with the same permissions on the same machines; Switchy's snapshot has the same sensitivity. Adding encryption requires a key-management story (where does the key live? is it per-device?) that is out of proportion for a two-account local use case.
  - macOS Keychain uses a different API surface (`security find-generic-password`) than the file-based capture the reference impl uses on Linux/WSL. Implementing both correctly in v1 doubles the test matrix for a platform the user does not run Switchy on daily.
- Consequence:
  - If a future Switchy release needs cross-machine snapshot sync or cloud backup, encryption becomes load-bearing and must be added as a migration (not a retrofit).
  - Adding macOS later is a v2 spec, not an implementation patch — the file-path abstractions and error surfaces must accommodate Keychain semantics.
- Spec pointer: `specs/official-multi-account/requirements.md` "Out of scope" + `specs/official-multi-account/design.md` D-7.

## 2026-04-07 - Rename forked app to Switchy and keep legacy deep-link compatibility

- Decision:
  - Rename the forked app to `Switchy` and switch the public deep-link scheme to `switchy://`, while still accepting legacy `ccswitch://`.
- Why:
  - The fork needed a distinct installed identity and visible branding.
  - Legacy deep-link compatibility avoids breaking existing links and upstream-style imports immediately.
- Consequence:
  - Installer/binary/app identifiers are now `Switchy`.
  - Visible branding is mostly `Switchy`.
  - Legacy deep links still function.
