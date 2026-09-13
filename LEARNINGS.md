# Learnings

Transferable heuristics captured from past sessions on this project. These are rules of thumb for future work — not history (that's `SESSION_LOG.md`) and not codified decisions (that's `DECISION_LOG.md`).

---

## Decide another tool's auth mode from the data you need, not from its mode field

Codex CLI stopped writing `auth_mode` into `~/.codex/auth.json`. Switchy's usage reader required `auth_mode == "chatgpt"` before it would touch the `tokens` block, so on every up-to-date install it concluded "no login" and hid the usage badges. There was no error and no log line: the feature quietly stopped existing, while the tokens it needed sat in the same file.

**Rule:** When reading another program's credential or config file, decide what you have from the data you are about to use — is there a `tokens` block with an access token? — and let an explicit mode field act only as a veto when it names a *different* mode. Optional discriminators get dropped between releases, and a strict equality check on one turns that into a silent outage. The same test applies to Claude's `.credentials.json`, Gemini's `oauth_creds.json` and Kimi's `credentials/kimi-code.json`.

---

## A test that reaches a machine-global path escapes `SWITCHY_TEST_HOME` and writes to real user data

The test home redirect sandboxes anything derived from `get_home_dir()`. It does not sandbox a path the app *discovers* — and `get_claude_mirror_override_dir()` falls back to `build_default_claude_mirror_dir()`, which probes for a WSL distro and returns a real `\\wsl$\...\home\<user>\.claude`. On any developer machine with WSL, every swap test therefore wrote credentials and identity into the actual WSL home. We hit this 2026-08-16: test fixtures (`uuid-B` / `bob@example.com` / `Acme`) ended up in `/home/agentcode/.claude.json`, and the blanked test credentials file was then healed by the running reconciler using the *Windows* login — silently moving that install to a different account. The tests all passed while doing it; three unrelated assertions had also been failing for months for the same reason, because "no mirror configured" tests were quietly running as mirror tests.

**Rule:** Any config value that is *auto-detected* rather than *derived from home* is a global side-channel and must be suppressed under the test-home env var, exactly like the macOS Keychain. Before trusting a hermetic-looking suite, grep the settings layer for fallbacks that probe the machine (`build_default_*`, `detect_*`, `which`, registry reads) and confirm each is gated. A cheap tell that you have this bug: tests that pass on CI but fail locally, or a suite whose outcome depends on whether a peer system happens to be installed. See [[DECISION_LOG]] 2026-08-16.

---

## `userID` in `.claude.json` identifies the *install*, not the account — despite looking like account state

`~/.claude.json` carries both machine state and account state as flat sibling keys, and they are not distinguishable by name. `userID` sits right next to `oauthAccount` and reads like an account attribute, but the **same account signed in on two machines carries a different `userID` on each** (registered2nd@gmail.com is `7aa2038f…` on Windows and `bb56a5f0…` in WSL). It behaves like `machineID`, not like `emailAddress`. Carrying it across an account swap stamps one install with another install's identity.

**Rule:** When splitting a flat config into "moves with the account" and "stays with the machine," do not classify by name or by plausibility — verify each candidate field against **the same account on two different installs**. Fields that differ there are install-scoped no matter what they are called. Fields already keyed by account or org UUID (`groveConfigCache`, `passesEligibilityCache`) are self-partitioning and need no classification at all; leaving them is safer than moving them.

---

## `CLAUDE_CONFIG_DIR` relocates Claude Code's whole global state, creating a second config root on one machine

Unset, Claude Code keeps `.claude.json` at the home root and everything else under `~/.claude/`. Set, it moves the lot — `.claude.json`, `.credentials.json`, `projects/`, `todos/`, `shell-snapshots/` — *inside* the named directory. Point it at `~/.claude` (the default directory, which looks like a no-op) and you get a **second, independent config root** at `~/.claude/.claude.json`, with its own login identity, alongside the default one. Both roots still share the credentials file and the backups folder, so it is not the account isolation it appears to be; the variable is undocumented and the issue asking what it does was closed with no answer. A tell that two roots exist: `~/.claude/backups/` holding `.claude.json.backup.*` files at two distinct sizes.

**Rule:** Any tool that reads or writes Claude Code's config must resolve the file by rule (`CLAUDE_CONFIG_DIR` if set, else home root), never by checking which candidate exists — a stale second root will win the probe forever. When two accounts must be genuinely separate, give each a separate home directory; `CLAUDE_CONFIG_DIR` splits too little to work.

---

## Claude Code stores its OAuth credentials in the macOS **Keychain**, not `~/.claude/.credentials.json`

On macOS, Claude Code keeps its login blob as a login-Keychain generic-password item (service `Claude Code-credentials`), **not** in the `~/.claude/.credentials.json` file it uses on Windows and Linux. A file-only code path silently no-ops on macOS: reads find nothing ("No login found" even when logged in), and writes land in a file Claude Code never reads (a swap that *looks* like it succeeded but doesn't take). Only the token blob moves to the Keychain — the `oauthAccount` block in `~/.claude/.claude.json` stays file-based on every platform.

**Rule:** For live Claude-cred read/write, abstract the store per-OS (`read_live_credentials` / `write_live_credentials` in `services/claude_account`). On macOS shell out to `security find-generic-password -s "Claude Code-credentials" -w` to read and `add-generic-password -U -s "Claude Code-credentials" -a <acct> -w <blob>` to update the current login **in place**. `services::subscription` already reads this item — reuse that pattern, don't re-derive. Tests must bypass the Keychain (it's a global side-channel `SWITCHY_TEST_HOME` can't sandbox). See [[DECISION_LOG]] 2026-07-05.

---

## pnpm 11 blocks dependency build scripts via `allowBuilds` — `onlyBuiltDependencies` is not enough

Building Switchy under pnpm 11.5.2 (corepack) fails twice before any compile:
1. **Non-TTY `node_modules` purge.** `pnpm tauri build` runs a deps-status-check that wants to purge+reinstall `node_modules`; with no TTY it aborts (`ERR_PNPM_ABORTED_REMOVE_MODULES_DIR_NO_TTY`).
2. **Blocked build scripts.** With `CI=true` set it proceeds but then fails (`ERR_PNPM_IGNORED_BUILDS`, exit 1) because `esbuild`/`msw` build scripts aren't approved. In this pnpm version the operative allowlist is the **`allowBuilds:` map** in `pnpm-workspace.yaml` (`esbuild: true` / `msw: true`), which pnpm auto-writes as a `set this to true or false` template — **not** `onlyBuiltDependencies` (that key is honored for already-approved deps like `@tailwindcss/oxide`, but adding entries to it does NOT clear an already-recorded ignored state). Approved scripts only run on an actual (re)install, so a clean reinstall / `--force` is needed for them to execute.

**Rule / recipe:** set `allowBuilds: { esbuild: true, msw: true }` in `pnpm-workspace.yaml`, then `CI=true pnpm install --force`, then `CI=true pnpm tauri build`. `CI=true` is required throughout (non-TTY).

---

## A running Claude Code session re-reads `.credentials.json` per request — a creds swap heals it live

A *running* Claude Code session does **not** cache its OAuth token in memory for the process lifetime — it re-reads `~/.claude/.credentials.json` per request. That's why Switchy's mid-session account switch takes effect immediately, and why writing a valid bundle back into a 401'd side recovers it without a restart. (GH issues claiming "running sessions never recover from a creds-file change" describe a narrower crashed-loop state and over-generalize.)

**Rule:** A file-layer credential fix CAN auto-heal a live session *if* the sync reaches the file the running process reads. So the failure mode to design against is wrong-*direction* sync (one-way leaving the other side stale), not "the process won't notice." Corollary: two long-lived Claude Code installs sharing one single-use rotating refresh token will always race — bidirectional health-aware sync narrows it; only separate logins or a single-refresher broker eliminates it. See [[DECISION_LOG]] 2026-06-13.

---

## `oauthAccount` is a label, not the credential — and Switchy reads it from `~/.claude/.claude.json` first

When diagnosing "Switchy captured/switched the wrong account," separate the two halves of a Claude credential:

- **The token** (`.credentials.json` → `accessToken`/`refreshToken`) *is* the account — it authenticates and spends. Identify an account by token sha, never by displayed name.
- **The `oauthAccount` block** (in `.claude.json`) is a cosmetic label (email/uuid/org for display). It can be stale or flat wrong relative to the token sitting beside it.

So: two snapshots with the **same name** but **different token shas** are two different accounts (the name was just mislabeled); the same token sha under two names is one account. Diagnose by sha.

Precedence trap that bit us 2026-06-09: Switchy's `live_claude_config_path()` (`services/claude_account/paths.rs:35`) reads `oauthAccount` from **`~/.claude/.claude.json` in preference to the home-root `~/.claude.json`**. Claude Code itself maintains the home-root file; `~/.claude/.claude.json` is often absent. If anything writes a stale/static `oauthAccount` into `~/.claude/.claude.json`, it **silently overrides the real login** for every `capture` (and `swap`'s `read_oauth_from_live`) — the captured *token* is correct but the recorded *name* is whatever that file says. Never hand-write `~/.claude/.claude.json` to "fix" identity; if a capture shows the wrong name, first check which of the two config files capture is reading.

---

## Compare AI gateway costs in real USD per workload mix — rate cards mislead Switchy preset selection

When evaluating gateways for Switchy presets (Aiberm, RightCode, bltcy/柏拉图AI, etc.) or recommending one in conversation, don't compare their published per-M token rates. Three sources of distortion stack and the ranking inverts depending on which you miss:

1. **Currency unit.** Some gateways display `$` that is real USD (Aiberm — confirmed via FX-anchored Alipay/PayPal top-up). Others display `$` as an RMB-pegged credit unit at top-up (RightCode currently 2 RMB per displayed-$ for Claude官渠, bltcy similar 1:1 / multi-tier setup). Same headline number can be 7× apart in real money.
2. **Multiplier chain.** Effective price = `base × 模型倍率 × 分组倍率`, with cache/output ratios pegged to Anthropic's standard (×0.1 cache read, ×1.25 5m cache write, ×5 output) on every gateway. Aiberm exposes the full chain in usage logs; bltcy via 5 named channel tiers per model; RightCode via per-channel `x2.00` markers. Rate-card "headline" numbers quote one point in this chain — usually the one most flattering for the gateway.
3. **Workload mix.** Cache-heavy continuations vs cache-cold first-prompts differ 30–100× per call on the same gateway. the user's actual Claude Code workload skews heavily cache-hit, so token-type weighting matters more than per-M headline.

**Rule:** When a user pastes a usage log, decode it: compute per-token-type real USD rate from one cache-dominant call, verify against a prompt-dominant call, weight by their actual mix. Don't trust rate-card headlines.

**Worked example (verified 2026-05-09 from Aiberm + RightCode usage logs, Sonnet 4.6):**

| Real USD per M | Aiberm Tier A | RightCode 官渠 (2:1) | Anthropic |
|---|---|---|---|
| Cache read | $0.056 | $0.085 | $0.30 |
| Prompt | $0.558 | $0.845 | $3.00 |
| Cache 5m write | $0.698 | $1.06 | $3.75 |
| Output | $2.789 | $4.23 | $15.00 |
| Vs Anthropic | -81% | -72% | baseline |
| Cross-gateway | — | ~34% more expensive than Aiberm | — |

Both deeply discounted vs Anthropic, but Aiberm Tier A consistently ~34% cheaper than RightCode in real USD on every token type. The "RightCode is 3× cheaper" framing (and any reverse) only appears if you mismatch units (treating displayed-RMB as real USD or applying a multiplier twice).

**How to apply:** When user shares a usage log, locate one mostly-cache-read call (cleanest sample). Compute `displayed_$ ÷ tokens × 1M`. Compare to `Anthropic_cache_rate × derived_multiplier`. If the ratio doesn't match across cache and prompt categories within the same call, the unit assumption is wrong — re-derive. Aiberm logs name the multiplier chain explicitly (`模型倍率 × 缓存倍率 × 分组倍率`); RightCode/bltcy logs leave it implicit and require the cross-token-type cross-check. Full method writeup: `C:\Projects\KnowledgeSystem\technology\chinese_ai_gateway_pricing_unit_and_multiplier_traps.md`.

## OAuth refresh tokens rotate single-use — sharing one credentials file across two stores guarantees one of them dies

When a credential file holds a rotating OAuth refresh token (Anthropic Claude Code, Google, most modern OAuth implementations), copying it to a second persistent store creates a race: each store's client will attempt its own refresh when its access token nears expiry, and only the first to succeed gets a valid new refresh_token. The server marks the old refresh_token as used; the second client's refresh comes back rejected; most clients then blank the refresh_token field to force re-auth. The two stores diverge silently — same bytes at copy time, different fates 8 hours later. We hit this 2026-05-06: switchy's swap-time mirror correctly delivered identical credentials to Win and WSL, both Claude Code instances ran independently, Win refreshed first (rotated R0→R1, persisted), WSL's later refresh against R0 was rejected, WSL's `.credentials.json` ended up with `refreshToken=""` and an expired access token — 401 with no recovery short of `claude /login`. The 2026-05-04 session had ruled out this race based on a 20-minute manual-cp re-test, but 20 minutes is inside access-token validity — neither side *needed* to refresh in that window, so the failure mode couldn't manifest. The race is a rotation collision, not a timing collision; multi-hour spans are required to observe it.

**Rule:** Don't dual-write OAuth credential files across filesystem boundaries unless you also dual-watch them. The instant the two stores have independent refresh chains, you're racing — and there's no way to detect "the other side already burned the refresh token" without sustained sync. Either (a) one source of truth with the second side reading through (proxy / network share / symlink), (b) one writer with a watcher mirroring strictly one-way (1.0.5's design — Win is the writer, WSL the consumer), or (c) bidirectional last-writer-wins watcher (more complex, more failure modes). Multiple terminals on one OS sharing one credential file is fine — they coordinate implicitly through the file. Two filesystems each with their own copy is not.

**How to apply:** When you find yourself writing the same secret-with-rotation to two paths, stop and ask "what watches the rotation?" If nothing does, you're shipping a bug that takes hours to surface. The Win→WSL case in switchy is the canonical example; the 1.0.5 fix is `services::credential_mirror` — a `notify`-crate FileSystemWatcher on the live dir that mirrors any `.credentials.json` change to the mirror path. When tempted to verify with a "copy then check 20 minutes later" test, force the failure window — observe across at least one access-token expiry boundary (~8 h for Anthropic Claude Code), not the post-copy quiet period.

## Repair stale Aiberm-relay sessions with JSONL surgery, not /clear

When a Switchy provider switch from Aiberm (or any relay backing a non-Anthropic model) to Anthropic Official triggers `Invalid signature in thinking block` or `Invalid data in redacted_thinking block` on `claude --resume`, the prior turns' content blocks were signed by an upstream the new provider can't validate. `/clear` works but loses context. Stripping the offending blocks from the session JSONL preserves the conversation. Different error messages (sometimes pointing at `tool_use`) can share the same root — sweep all thinking-shape blocks, not just the variant the error names.

**Rule:** When this error fires, run `node C:\Projects\Switchy\strip-thinking.mjs <session.jsonl>`, fully quit Claude Code, then `claude --resume`. The script drops `thinking` and `redacted_thinking` content blocks and rewrites the `parentUuid` chain so the conversation graph stays intact — without that rewrite, the resumed agent loses everything before the first dropped line.

**How to apply:** Find the session file by mtime in the project's `~/.claude/projects/<encoded-cwd>/` dir. Verify with `grep '"signature":""'` (Aiberm fingerprint) or `grep '"type":"redacted_thinking"'`. Script backs up to `.bak.jsonl`. **Never `cp bak src` to "restore and re-strip" while Claude Code is still attached** — the bak is a snapshot from before the running session appended its in-flight work, and the cp will obliterate that work; strip in place. Passthrough relays like RightCode don't trigger this (same Anthropic signing keys on both sides of the switch), so the failure is specific to wrapping-style relays.

## Claude Code reads OAuth account identity from `~/.claude.json` (home root), not `~/.claude/.claude.json`

When mirroring credentials to give Claude Code a different account (e.g. WSL mirror after a Switchy swap), updating `~/.claude/.credentials.json` and `~/.claude/.claude.json` is not sufficient — the `oauthAccount` block read by `claude auth status` lives in `~/.claude.json` at the home root (the 85KB main config file with `numStartups`, `tipsHistory`, etc.). Same OAuth access token can resolve to different accounts on different machines purely because of which `.claude.json` is being read. We hit this 2026-05-02: identical tokens on Windows + WSL, but `claude auth status` on Windows showed the just-switched Apple ID account while WSL kept showing the previous `registered2nd@gmail.com`. The mirror was writing `~/.claude/.claude.json`; Claude Code on WSL was reading `~/.claude.json`.

**Rule:** Any code that swaps Claude Code's effective identity must update `~/.claude.json` (home root) when it exists, not just files inside `~/.claude/`. On Windows the home-root file is the only one — Switchy's `live_claude_config_path` already falls through to it. On Linux/WSL the dir-prefixed file *also* exists from earlier writes, and the dir-only update silently leaves identity stale.

**How to apply:** Mirror writes should walk both candidates. After updating `mirror_dir/.claude.json`, check `mirror_dir.parent()/.claude.json` and update its `oauthAccount` block too. Implemented in `services/claude_account/mod.rs:411` after the primary mirror write — `fs::read` → `merge::replace_oauth_account` → `store::write_snapshot_atomic`. Best-effort, log-warn-on-failure (do not fail the swap on home-root sync errors).

## React Query: when component-level queries flip on an `isCurrent`-style flag, invalidate ALL the affected query keys after the flip

Pattern: a component picks between `liveQuery` (no id, reads "current" state) and `perEntityQuery(id)` (reads snapshot for that id) based on whether the component owns the currently-active entity. When you mutate the active entity (A → B), both cards re-render: A switches from liveQuery to perEntityQuery(A), and B switches from perEntityQuery(B) to liveQuery. **The live query cache still has data tied to A.** With `staleTime > 0`, B will display A's data from the live cache for up to staleTime ms before any refetch fires. Hit 2026-05-02 with the subscription-quota cards: switching from account A to B showed A's quota on B's card.

**Rule:** When a mutation changes which entity owns "current," invalidate every query key the affected components might switch to or from — not just the entity list. Specifically: invalidate the live query AND any per-entity queries with the same data shape, since both might now be displaying the wrong row.

**How to apply:** In the mutation's `onSuccess`, add `invalidateQueries({ queryKey: [data-shape-prefix] })` covering the whole namespace. Switchy's fix: `await queryClient.invalidateQueries({ queryKey: ["subscription", "quota"] })` in `useSwitchProviderMutation` — invalidates both `["subscription", "quota", appId]` (live) and `["subscription", "quota", "provider", providerId]` (per-provider) in one call. The prefix-only key forces a refetch on every mounted card.

## Z-index doesn't resolve "same opaque pixels"

When two UI elements are anchored to the same spot and one has an opaque background (`bg-card/95`, `backdrop-blur`, whatever), no layering order makes both usable. Whichever sits on top, the other is visually hidden and unreachable. The upstream `ProviderCard` deliberately overlaid the hover-action strip on top of the quota pill — that was the design. Fighting it by promoting the pill above didn't fix the collision; it just flipped which element became unreachable.

**Rule:** Before reaching for `z-index`, describe where each element is anchored in the DOM and ask whether they occupy the same pixels. If yes, the fix is structural — put them side-by-side in the flex row, or swap one out for the other on state change. Stacking only helps when the elements occupy different pixels and the question is which lid sits above a sticky-out bit.

**How to apply:** When an "overlay on hover" design collides with an always-visible element, reach for `hidden group-hover:flex` on an in-flow sibling before reaching for `absolute + z-index`. The sibling approach never overlaps and never needs `bg-*` to hide the element behind it.

## Check written ToS before reference implementations

When planning to call a third-party endpoint that reference clients (sub2api, claude-relay-service, hermes-agent, opencode) are already calling, the existence of those clients is *not* evidence the call is permitted. A policy can explicitly prohibit what reference clients do — and frequently does, because the policy was written *in response to* those clients. BACKLOG #5 survived a full requirements review and only died when a late web search surfaced Anthropic's April 2026 "Authentication and credential use" policy, which names the exact pattern as a ToS violation. If we had checked the policy page first, the spec loop wouldn't have been written at all.

**Rule:** Before writing requirements for any feature that authenticates to a vendor's endpoint from a client the vendor didn't write, read the current ToS / credential-use policy page *first*. Reference-client existence is weak evidence; written policy is strong.

**How to apply:** On the "should we build this?" question for any feature that touches vendor auth endpoints, the order is (a) policy page, (b) endpoint behavior, (c) reference implementations. Inverting that order — starting from "look, other people do this" — repeatedly produces specs that die at the last minute, after real work has already been spent.

## Tauri's "incremental build" doesn't notice icon source changes

`pnpm tauri build` will happily reuse a cached compiled resource when the source `.ico`/`.png` files change. The build appears to succeed, the new icon is on disk, but the embedded resource in `switchy.exe` is stale. We hit this in the 2026-04-19 1.0.0 ship: the icon redesign committed in `605e0412` shipped with the old icon embedded in the binary because the rebuild was incremental.

**Rule:** When changing icons (or any embedded resource), `cargo clean` first, then `pnpm tauri build`. The 8-15 min penalty is worth not shipping the wrong icon.

**How to apply:** `cd src-tauri && cargo clean && cd .. && pnpm tauri build`. To verify the new exe before shipping: `[System.Drawing.Icon]::ExtractAssociatedIcon('path\to\switchy.exe').ToBitmap().Save('verify.png')` from PowerShell, then look at the PNG.

## NSIS bundler's "mis-hashed files" error means manual DLL replacement

Tauri's NSIS bundler validates its plugin DLL cache (`%LOCALAPPDATA%\tauri\NSIS\Plugins\x86-unicode\additional\nsis_tauri_utils.dll`) against an expected hash. If the cache file's hash doesn't match (because Tauri updated which upstream commit it pins, or the cache is from an older Tauri version), the bundler tries to redownload from `https://github.com/tauri-apps/nsis-tauri-utils/releases/...`. From this network, that download routinely times out — fails with `failed to bundle project: 'timeout: global'`.

**Rule:** When you see `NSIS directory contains mis-hashed files. Redownloading them.` followed by the global-timeout error, don't keep retrying the bundler. Manually download the DLL once and cp it into the cache slot.

**How to apply:**
```bash
curl -L --max-time 300 -o /tmp/nsis_tauri_utils.dll \
  "https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.1/nsis_tauri_utils.dll"
cp /tmp/nsis_tauri_utils.dll \
  "$LOCALAPPDATA/tauri/NSIS/Plugins/x86-unicode/additional/nsis_tauri_utils.dll"
pnpm tauri build --bundles nsis
```
Version pinned in the URL (`v0.5.1` here) needs to match what the current Tauri CLI expects — bump it if Tauri prints a different version in the "Downloading..." line. The MSI bundle is unaffected by this issue, so if you only need the MSI, `--bundles msi` skips it entirely.

## Windows shell icon cache survives explorer restart

Changing an app's icon and rebuilding the installer is not enough for the new icon to show up in Explorer / taskbar / Start menu. Windows caches shell icons in `iconcache.db` + `thumbcache_*.db` under `%LocalAppData%\Microsoft\Windows\Explorer\`. Those files survive `explorer.exe` restart — they're closed when explorer exits and re-read when it starts, and explorer does not invalidate them based on the source icon changing. The user sees the old icon and assumes the build is wrong.

**Rule:** When reporting "my new icon isn't showing," the installer build probably has the right icon. The fix is to actually clear the shell icon cache, not rebuild.

**How to apply:** The minimal incantation depends on the shell — the user's default Windows shell is PowerShell, so lead with that. If the user is on Git Bash instead, the `rm -f` / `//f` form works but will fail noisily in PowerShell (ambiguous `-f` between `-Filter`/`-Force`; double-slash gets passed literally to `taskkill`).

PowerShell:
```powershell
taskkill /f /im explorer.exe
Remove-Item -Force "$env:LOCALAPPDATA\Microsoft\Windows\Explorer\iconcache*"
Remove-Item -Force "$env:LOCALAPPDATA\Microsoft\Windows\Explorer\thumbcache_*.db"
Start-Process explorer.exe
```

Git Bash:
```bash
taskkill //f //im explorer.exe
rm -f "$LOCALAPPDATA/Microsoft/Windows/Explorer/iconcache"*
rm -f "$LOCALAPPDATA/Microsoft/Windows/Explorer/thumbcache_"*.db
start explorer.exe
```

Softer alternative (either shell): `ie4uinit.exe -show` — sometimes enough, no explorer restart needed. If the icon is wrong *inside* the app window (title bar, tray), that's a build problem; if it's only wrong in Explorer/taskbar/Start menu shortcuts, it's the shell cache.

## I18n default language: pick English, not the developer's language

Fresh `settings.json` files don't have a `language` field. Whatever `unwrap_or("...")` you pick becomes the default for every user who hasn't explicitly set it. Upstream `cc-switch` defaulted to `"zh"` because that was the developer's language; Switchy inherited that default and shipped a Chinese tray menu to English users on fresh installs. Frontend i18n (react-i18next) has browser-language detection that masks this; Rust-side defaults (tray menu, system dialogs) do not, so they silently fall through to whatever the fallback arm is.

**Rule:** For any `match language { "en" => ..., "ja" => ..., _ => ... }` default arm in Rust code, the `_` should be English, not Chinese. Browser i18n detection doesn't reach Rust.

**How to apply:** When auditing i18n coverage, grep Rust for `language.as_deref().unwrap_or(` and `match .* language` — those fall-through arms are the ones end-users never see until they complain about a foreign-language UI element.

## Don't chain bug-fix attempts without tracing the failure each time

In the overlay-z-index thread, four commits tried to fix the same bug by moving things around (top-2 → bottom-2 → z-20-on-parent → z-20-on-child) without re-examining why each prior attempt failed. Each attempt rebuilt, reshipped, retested with the user — a 5-minute cycle four times over. The correct fix took one commit once the geometry was actually traced.

**Rule:** If the first attempt at a visual bug doesn't work, don't try a variant. Stop and ask: what does the DOM/CSS actually produce here, and why did the fix not take? One round of "let me look at the code again" beats three rounds of "let me try moving it."

**How to apply:** On any rebuild that takes >5 min (Tauri release builds, native compiles), the cost of being wrong compounds. Trace before you tweak.
