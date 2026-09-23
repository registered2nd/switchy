# Switchy

Switchy switches and pools accounts for four command-line coding tools: **Claude Code, Codex, Gemini CLI and Kimi Code**. Each tool gets a list of providers — subscription logins or API keys — and Switchy writes the one you pick into that tool's own config. Route a tool through Switchy's local proxy and several subscription accounts serve it as one pool, moving to the next account before the current one reaches its limit.

It is a desktop app for Windows, built with Tauri; it also builds for macOS. Release history is in [CHANGELOG.md](CHANGELOG.md).

## Providers

Each tool has its own list of provider cards. Add a provider from a preset or write a custom config; on first launch, *Import Current Config* saves the tool's existing config as a default provider.

*Enable* on a card, or picking the provider from the tray, writes it into the tool's config. Claude Code picks up a switch without a restart. Codex does too while it is routed through the proxy (see [Switching Codex accounts in an open session](#switching-codex-accounts-in-an-open-session)); otherwise exit and run `codex resume --last`, which brings the conversation back under the new account. Gemini CLI and Kimi Code need a restart.

Settings other than the key and endpoint — plugins, hooks, permissions — go in the tool's common config snippet (*Edit common config* in the provider form). It is merged into every provider that has *Write common config* checked, so switching providers does not drop them. Switching between Claude Official accounts changes only the connection settings in `settings.json` and leaves the rest of the file as it is.

## Official accounts

An Official provider holds a subscription login instead of an API key, so switching providers switches accounts.

- **Claude Code.** Sign in with `claude /login` while the Official provider is current, then click *Capture current account* on its card. Switchy stores that login with the card and puts it back whenever the card is enabled.
- **Codex and Kimi Code.** Each Official card keeps the login you signed in with while it was current; there is nothing to capture. To add a Codex account, add an OpenAI Official provider, enable it, and run `codex login`.
- **Usage badges.** An Official card shows the subscription's quota: how much of each window is used and when it resets. What Gemini and Kimi cards can show is under [Known limitations](#what-a-gemini-or-kimi-card-can-show).

## The pool

Settings → Pool holds everything the pool runs on:

- **Local proxy.** Routing, switching and rotation all need it running.
- **Route through the proxy.** The apps whose requests go through the proxy.
- **Switch automatically.** Serves an app from its switching order instead of the one provider you picked: it starts at the top and moves to the next when a request fails. *Enable* on a card (or picking an account in the tray) holds that account for 10 minutes: requests go only to it, with no failover or rotation, so a broken account shows its error. After that, automatic switching resumes.
- **Rotate accounts before the limit.** Moves an account whose usage has reached the threshold to the back of the order until its limit resets.
- **Keep accounts warm.** Opens each account's session window in advance.
- **Global Outbound Proxy.** The proxy Switchy itself uses to reach the services.

### Switching Codex accounts in an open session

A running Codex session reads its ChatGPT login once, at start, and refuses to reload a login that belongs to a different account. Switching the Official provider therefore changes the account for the next session, not the open one.

Turn on the proxy for Codex (Settings → Pool → Route through the proxy) and the open session switches too. Codex keeps its own login and its built-in provider is pointed at the local proxy; on every request the proxy replaces that login with the one of the Official provider currently selected. Codex stays in ChatGPT mode, so its model list and its `codex resume` history are unchanged. Sessions started before the proxy was turned on keep talking to OpenAI directly until they are restarted.

- **Rotation.** With *Switch automatically* on, the app's switching order is the pool of accounts. *Enable* on a card still picks an account by hand: it puts that account first in the switching order, and it answers from the next request on. *Rotate accounts before the limit* moves an account whose usage has reached the threshold to the back of the order until its limit resets, and an account OpenAI refuses for usage is passed over until the reset it names. When every account is spent the request still goes out, so you see OpenAI's own message.
- **Logins stay usable.** The proxy renews a stored login when its access token runs out and hands the renewed login to Codex's own `auth.json` and to the copy restored when the proxy is turned off, under the same newest-valid-login-wins rules as [One login, two installs](#one-login-two-installs). A login OpenAI has rejected is not retried; sign in again with `codex login` while that provider is current. A login made with `codex login` is stored with the enabled card, with the proxy on or off.
- **Exit check.** Before a stored login is used, the proxy asks the edge in front of the service (`chatgpt.com`, or `api.anthropic.com` for Claude) where it sees this machine, over the route the request will take. If that is mainland China, or the question gets no answer, the request is held for up to 30 seconds and then refused; nothing is sent. Set the Global Outbound Proxy if this machine reaches the internet through a local proxy port rather than a router or TUN path.
- **WSL.** The Codex install in WSL is pointed at the proxy too, so its sessions switch accounts the same way. See [WSL](#wsl) for what that needs. It must be signed in with ChatGPT. When the proxy is turned off it gets the login of the account that is current then.

### Rotating Claude accounts through the proxy

Claude Code picks up a switch without a restart, so Official Claude accounts already swap by hand at any time. Rotating them automatically goes through the proxy: turn on routing for Claude (Settings → Pool → Route through the proxy). An Official account whose login has not been captured cannot be served, so Switchy refuses the takeover and says so; capture it from the provider card first. The Claude install in WSL is pointed at the proxy too, under the same conditions as Codex. Claude Code sessions started while the proxy was on keep calling it after it is turned off; quit and reopen them.

With the proxy taken over for Claude, Claude Code keeps its subscription sign-in and only its API address changes. Enabling an account, by hand or by rotation, also puts that account's captured login into Claude Code's saved login, as a switch with the proxy off does, so `/status` names the account that serves the requests. The proxy presents the same account's captured login on each request and patches the account id Claude Code writes into the request to match it. Rotation, renewal, the exit check and the login rules are the ones described for Codex above. Anthropic's rate-limit headers decide when an account is spent, per model: some models have a weekly limit of their own on top of the account's shared windows (Fable does; Opus and Sonnet share the account's), so an account can be spent for Fable and still serve Opus. An account Anthropic refuses is passed over until the reset it names, for every model when the shared windows refused and for that model alone when its own limit did. Which limit belongs to which model is read from Anthropic's answers, so it is known once an account has answered a request for that model. Claude Code's own identity calls (`/api/oauth/*`) pass through with the login Claude Code sent, which is the enabled account's.

Switchy then sits in the request path and renews captured logins itself, using Claude Code's own client id against Anthropic's token endpoint. Whether that fits Anthropic's terms for subscription logins is yours to weigh before turning the proxy on for Claude.

### Keeping pooled accounts warm

A subscription's session window — Anthropic's five hours, ChatGPT's equivalent — opens on a real request and resets a fixed time later. An account nobody has used has no window running, so when rotation moves onto it the window starts from cold, and that account is the one holding the session with its reset furthest away.

*Keep accounts warm* opens those windows in advance. Each Official account is looked at on the interval you set, and one request for a single token goes out to the ones whose window has lapsed. An account whose window is still running is skipped, and so is one at its limit, so the cost stays near one request per account per window. It runs while Switchy is running and needs neither the proxy nor rotation to be on; it is off by default, because it spends quota with nobody present.

Presenting the login renews it, so a warmed account is also a signed-in one — but only as far as its refresh token reaches. That deadline is set by the last sign-in in the browser and renewing does not push it back, so keep-warm cannot hold an account open indefinitely; when the refresh token expires the account needs `claude /login` or `codex login` while it is the current provider. Everything else is as it is for a real request: the exit check runs first, a rejected refresh token is not retried, and the quota the answer reports is recorded for rotation to use — which is the other thing keep-warm buys, since an account nobody has used reports nothing at all.

The model each request uses is the test model for that tool under Settings → Advanced → Model Test Config, which defaults to the cheapest one. A Codex account signed in with ChatGPT is refused API models, so it gets the test model only when Codex lists that model for ChatGPT logins, and otherwise the model Codex lists last, at its lightest effort.

## WSL

Settings → Advanced → Configuration Directory has a *Claude Code Mirror Directory* and a *Codex Mirror Directory*. Point one at the WSL install's config directory (for example `\\wsl$\Ubuntu\home\<user>\.claude`) and every switch is written there too, so both sides stay on the same provider and account. The Claude mirror updates only the provider fields and keeps the WSL side's own hooks, status line and plugins; [docs/fork_notes_windows_wsl_claude_sync.md](docs/fork_notes_windows_wsl_claude_sync.md) has the details.

While an app is routed through the proxy, its WSL install is pointed at the proxy as well. WSL reaches the proxy only with mirrored networking (`networkingMode=mirrored` under `[wsl2]` in `%USERPROFILE%\.wslconfig`); without it the WSL install is left alone.

## Handing configs back

Routing an app through the proxy edits its config — Claude Code's `settings.json`, Codex's `config.toml` — to point at the proxy. When the proxy lets go, whether it is turned off, Switchy quits, or Switchy starts again after a crash, only what the proxy changed is put back. Hooks, plugins and settings that another tool or you added in the meantime stay, such as the status hooks Orca writes. Switching the Claude account while the proxy is on keeps them too.

## Other tools in the app

- **Session Recovery** (Settings → Advanced): finds Claude Code transcripts that fail to resume after a switch because they carry thinking blocks the new account or endpoint cannot validate. Repair strips those blocks and relinks the conversation, keeping a backup of the original file.
- **Usage Statistics** (a tab in Settings): per account, the requests, tokens and rate-limit refusals that went through the proxy and when it was last used; and a history of every account switch with its reason (picked by hand, a failed request, a usage limit, a refused login, rotation). Successful requests are recorded only while request logging is on (Settings → Pool → Proxy server).
- **Health check**: the test button on a provider card sends it one request to see that it answers. The models used are set under Settings → Advanced → Model Test Config.
- **Environment variable conflicts**: a banner lists environment variables whose names contain `ANTHROPIC`, `OPENAI` or `GEMINI`, since they can override the config Switchy writes, and can delete them after backing them up.
- **Tray**: switch providers without opening the window.
- **Backups and moving to another machine**: the database is backed up on a schedule (Settings → Advanced → Backup & Restore), and Settings → Advanced → Data Management exports and imports it as SQL. That export is how providers move to another machine. Sign each machine in to each account itself rather than copying a login across: one login on two machines races on renewal (see [One login, two installs](#one-login-two-installs)).

## Setup

**Windows.** Run `installers/Switchy_<version>_x64-setup.exe` or `installers/Switchy_<version>_x64_en-US.msi`, or build from source (below).

**macOS.** The workflow in [.github/workflows/build-macos.yml](.github/workflows/build-macos.yml) builds an Apple Silicon `.dmg` when run by hand from the Actions tab. macOS blocks the first launch; right-click the app and choose Open.

**Data** lives in `~/.switchy/`:

- `switchy.db` — providers and app settings (SQLite)
- `settings.json` — device-level preferences
- `backups/` — database backups
- `accounts/` — captured Claude logins, one directory per provider

## Development

Needs Node 22 (see `.node-version`), pnpm and Rust 1.85 or later.

```bash
pnpm install          # dependencies
pnpm dev              # run the app with hot reload
pnpm typecheck        # TypeScript check
pnpm test:unit        # frontend tests (vitest)
cd src-tauri && cargo test   # backend tests
```

`CI=true pnpm build` builds the app and copies the current version's `-setup.exe` and `.msi` into `installers/`. `CI=true` stops pnpm from prompting mid-build, which fails when no terminal is attached. Under pnpm 11 a fresh checkout may also need `CI=true pnpm install --force` so the `esbuild` and `msw` build scripts allowed in `pnpm-workspace.yaml` actually run; [LEARNINGS.md](LEARNINGS.md) has this and the other build notes.

The frontend is React and TypeScript (`src/`), the backend Tauri and Rust (`src-tauri/`). The SQLite database is the store; switching writes the selected provider into each tool's own config files.

## Known limitations

### Claude accounts need a browser re-login every few weeks

Switchy can capture an Official Claude provider's login and swap between multiple accounts on demand, including accounts you have not touched for days.

Two deadlines govern how long a captured account stays usable. The access token expires in about 8 hours and is renewed automatically. The refresh token behind it has its own expiry, typically one to several weeks out, and that deadline is anchored to when you originally signed in through the browser — renewing does not push it back. Once it passes, that account needs `claude /login` again; no local state management extends it.

### One login, two installs

Claude Code and Codex both rotate the refresh token on every renewal, and the server accepts each one only once. Two installs holding the same login — most commonly Windows and WSL — will therefore race, and the one that renews second is rejected and left signed out.

Switchy reconciles the two sides in the background, moving the surviving login to whichever side lost, so this heals on its own. It needs the mirror directory configured and reachable (one per tool, under Settings → Advanced → Configuration Directory); while WSL is shut down, a rotation that happens on the Windows side cannot be propagated until it comes back. For Codex, an install that is on an API key rather than a ChatGPT login is left alone.

### What a Gemini or Kimi card can show

Usage badges and account switching both come from signing a tool in with an account. Claude Code, Codex and Kimi Code keep that login in a file Switchy stores with each provider, so switching providers switches accounts.

Gemini CLI is different. A Gemini provider holds the API key and endpoint, not the Google sign-in, so switching Gemini providers never changes which Google account is signed in. Gemini usage badges appear only while Gemini CLI is signed in with Google (`/auth` → Login with Google); on an API key there is nothing to show.

Kimi Code publishes no usage figures at all, so Kimi cards never show badges.

## License

MIT — Switchy began as a fork of cc-switch by Jason Young. See [LICENSE](LICENSE).
