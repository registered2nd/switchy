<div align="center">

# Switchy

### The All-in-One Manager for Claude Code, Codex, Gemini CLI, Kimi Code, OpenCode & OpenClaw

[![Version](https://img.shields.io/github/v/release/registered2nd/switchy?color=blue&label=version)](https://github.com/registered2nd/switchy/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/registered2nd/switchy/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Downloads](https://img.shields.io/github/downloads/registered2nd/switchy/total)](https://github.com/registered2nd/switchy/releases/latest)

English | [中文](README_ZH.md) | [日本語](README_JA.md) | [Changelog](CHANGELOG.md)

</div>


## Why Switchy?

Modern AI-powered coding relies on CLI tools like Claude Code, Codex, Gemini CLI, Kimi Code, OpenCode, and OpenClaw — but each has its own configuration format. Switching API providers means manually editing JSON, TOML, or `.env` files, and there is no unified way to manage MCP and Skills across multiple tools.

**Switchy** gives you a single desktop app to manage all five CLI tools. Instead of editing config files by hand, you get a visual interface to import providers with one click, switch between them instantly, with 50+ built-in provider presets, unified MCP and Skills management, and system tray quick switching — all backed by a reliable SQLite database with atomic writes that protect your configs from corruption.

- **One App, Six CLI Tools** — Manage Claude Code, Codex, Gemini CLI, Kimi Code, OpenCode, and OpenClaw from a single interface

## Fork Notes

This fork contains a Windows + WSL Claude mirror feature that is not part of upstream behavior. See [docs/fork_notes_windows_wsl_claude_sync.md](docs/fork_notes_windows_wsl_claude_sync.md) for the exact semantics and caveats.
- **No More Manual Editing** — 50+ provider presets including AWS Bedrock, NVIDIA NIM, and community relays; just pick and switch
- **Unified MCP & Skills Management** — One panel to manage MCP servers and Skills across four apps with bidirectional sync
- **System Tray Quick Switch** — Switch providers instantly from the tray menu, no need to open the full app
- **Cloud Sync** — Sync provider data across devices via Dropbox, OneDrive, iCloud, or WebDAV servers
- **Cross-Platform** — Native desktop app for Windows, macOS, and Linux, built with Tauri 2
- **Built-in Utilities** — Includes various utilities for first-launch login confirmation, signature bypass, plugin extension sync, and more

## Screenshots

|                  Main Interface                   |                  Add Provider                  |
| :-----------------------------------------------: | :--------------------------------------------: |
| ![Main Interface](assets/screenshots/main-en.png) | ![Add Provider](assets/screenshots/add-en.png) |

## Features

[Full Changelog](CHANGELOG.md) | [Release Notes](docs/release-notes/v3.12.3-en.md)

### Provider Management

- **6 CLI tools, 50+ presets** — Claude Code, Codex, Gemini CLI, Kimi Code, OpenCode, OpenClaw; copy your key and import with one click
- **Universal providers** — One config syncs to multiple apps (OpenCode, OpenClaw)
- One-click switching, system tray quick access, drag-and-drop sorting, import/export

### Proxy & Failover

- **Local proxy with hot-switching** — Format conversion, auto-failover, circuit breaker, provider health monitoring, and request rectifier
- **App-level takeover** — Independently proxy Claude, Codex, or Gemini, down to individual providers

### MCP, Prompts & Skills

- **Unified MCP panel** — Manage MCP servers across 4 apps with bidirectional sync and Deep Link import
- **Prompts** — Markdown editor with cross-app sync (CLAUDE.md / AGENTS.md / GEMINI.md) and backfill protection
- **Skills** — One-click install from GitHub repos or ZIP files, custom repository management, with symlink and file copy support

### Usage & Cost Tracking

- **Usage dashboard** — Track spending, requests, and tokens with trend charts, detailed request logs, and custom per-model pricing

### Session Manager & Workspace

- Browse, search, and restore conversation history across all apps
- **Workspace editor** (OpenClaw) — Edit agent files (AGENTS.md, SOUL.md, etc.) with Markdown preview

### System & Platform

- **Cloud sync** — Custom config directory (Dropbox, OneDrive, iCloud, NAS) and WebDAV server sync
- **Deep Link** (`switchy://`) — Import providers, MCP servers, prompts, and skills via URL
- Dark / Light / System theme, auto-launch, auto-updater, atomic writes, auto-backups, i18n (zh/en/ja)

## FAQ

<details>
<summary><strong>Which AI CLI tools does Switchy support?</strong></summary>

Switchy supports six tools: **Claude Code**, **Codex**, **Gemini CLI**, **Kimi Code**, **OpenCode**, and **OpenClaw** (OpenClaw's tab is off by default; enable it under Settings → App visibility). Each tool has dedicated provider presets and configuration management.

</details>

<details>
<summary><strong>Do I need to restart the terminal after switching providers?</strong></summary>

For most tools, yes — restart your terminal or the CLI tool for changes to take effect. **Claude Code** picks up a switch without a restart. **Codex** does too while it is routed through the local proxy (see [Switching Codex accounts in an open session](#switching-codex-accounts-in-an-open-session)); otherwise exit and run `codex resume --last`, which brings the conversation back under the new account.

</details>

<details>
<summary><strong>My plugin configuration disappeared after switching providers — what happened?</strong></summary>

Switchy provides a "Shared Config Snippet" feature to pass common data (beyond API keys and endpoints) between providers. Go to "Edit Provider" → "Shared Config Panel" → click "Extract from Current Provider" to save all common data. When creating a new provider, check "Write Shared Config" (enabled by default) to include plugin data in the new provider. All your configuration items are preserved in the default provider imported when you first launched the app.

</details>

<details>
<summary><strong>macOS installation</strong></summary>

Switchy for macOS is code-signed and notarized by Apple. You can download and install it directly — no extra steps needed. We recommend using the `.dmg` installer.

</details>

<details>
<summary><strong>Why can't I delete the currently active provider?</strong></summary>

Switchy follows a "minimal intrusion" design principle — even if you uninstall the app, your CLI tools will continue to work normally. The system always keeps one active configuration, because deleting all configurations would make the corresponding CLI tool unusable. If you rarely use a specific CLI tool, you can hide it in Settings. To switch back to official login, see the next question.

</details>

<details>
<summary><strong>How do I switch back to official login?</strong></summary>

Add an official provider from the preset list. After switching to it, run the Log out / Log in flow, and then you can freely switch between the official provider and third-party providers. Codex supports switching between different official providers, making it easy to switch between multiple Plus or Team accounts: each official Codex provider keeps the ChatGPT login you signed in with while it was active, and its card shows that account's email, plan and usage.

</details>

<details>
<summary><strong>Where is my data stored?</strong></summary>

- **Database**: `~/.switchy/switchy.db` (SQLite — providers, MCP, prompts, skills)
- **Local settings**: `~/.switchy/settings.json` (device-level UI preferences)
- **Backups**: `~/.switchy/backups/` (auto-rotated, keeps 10 most recent)
- **Skills**: `~/.switchy/skills/` (symlinked to corresponding apps by default)
- **Skill Backups**: `~/.switchy/skill-backups/` (created automatically before uninstall, keeps 20 most recent)

</details>

## Documentation

For detailed guides on every feature, check out the **[User Manual](docs/user-manual/en/README.md)** — covering provider management, MCP/Prompts/Skills, proxy & failover, and more.

## Quick Start

### Basic Usage

1. **Add Provider**: Click "Add Provider" → Choose a preset or create custom configuration
2. **Switch Provider**:
   - Main UI: Select provider → Click "Enable"
   - System Tray: Click provider name directly (instant effect)
3. **Takes Effect**: Restart your terminal or the corresponding CLI tool to apply changes (Claude Code does not require a restart)
4. **Back to Official**: Add an "Official Login" preset, restart the CLI tool, then follow its login/OAuth flow

### MCP, Prompts, Skills & Sessions

- **MCP**: Click the "MCP" button → Add servers via templates or custom config → Toggle per-app sync
- **Prompts**: Click "Prompts" → Create presets with Markdown editor → Activate to sync to live files
- **Skills**: Click "Skills" → Browse GitHub repos → One-click install to all apps
- **Sessions**: Click "Sessions" → Browse, search, and restore conversation history across all apps

> **Note**: On first launch, you can manually import existing CLI tool configs as the default provider.

## Download & Installation

### System Requirements

- **Windows**: Windows 10 and above
- **macOS**: macOS 12 (Monterey) and above
- **Linux**: Ubuntu 22.04+ / Debian 11+ / Fedora 34+ and other mainstream distributions

### Windows Users

Download the latest `Switchy-v{version}-Windows.msi` installer or `Switchy-v{version}-Windows-Portable.zip` portable version from the [Releases](../../releases) page.

### macOS Users

**Method 1: Install via Homebrew (Recommended)**

```bash
brew tap registered2nd/switchy
brew install --cask switchy
```

Update:

```bash
brew upgrade --cask switchy
```

**Method 2: Manual Download**

Download `Switchy-v{version}-macOS.dmg` (recommended) or `.zip` from the [Releases](../../releases) page.

> **Note**: Switchy for macOS is code-signed and notarized by Apple. You can install and open it directly.

### Arch Linux Users

**Install via paru (Recommended)**

```bash
paru -S switchy-bin
```

### Linux Users

Download the latest Linux build from the [Releases](../../releases) page:

- `Switchy-v{version}-Linux.deb` (Debian/Ubuntu)
- `Switchy-v{version}-Linux.rpm` (Fedora/RHEL/openSUSE)
- `Switchy-v{version}-Linux.AppImage` (Universal)

> **Flatpak**: Not included in official releases. You can build it yourself from the `.deb` — see [`flatpak/README.md`](flatpak/README.md) for instructions.

<details>
<summary><strong>Architecture Overview</strong></summary>

### Design Principles

```
┌─────────────────────────────────────────────────────────────┐
│                    Frontend (React + TS)                    │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────────┐    │
│  │ Components  │  │    Hooks     │  │  TanStack Query  │    │
│  │   (UI)      │──│ (Bus. Logic) │──│   (Cache/Sync)   │    │
│  └─────────────┘  └──────────────┘  └──────────────────┘    │
└────────────────────────┬────────────────────────────────────┘
                         │ Tauri IPC
┌────────────────────────▼────────────────────────────────────┐
│                  Backend (Tauri + Rust)                     │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────────┐    │
│  │  Commands   │  │   Services   │  │  Models/Config   │    │
│  │ (API Layer) │──│ (Bus. Layer) │──│     (Data)       │    │
│  └─────────────┘  └──────────────┘  └──────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

**Core Design Patterns**

- **SSOT** (Single Source of Truth): All data stored in `~/.switchy/switchy.db` (SQLite)
- **Dual-layer Storage**: SQLite for syncable data, JSON for device-level settings
- **Dual-way Sync**: Write to live files on switch, backfill from live when editing active provider
- **Atomic Writes**: Temp file + rename pattern prevents config corruption
- **Concurrency Safe**: Mutex-protected database connection avoids race conditions
- **Layered Architecture**: Clear separation (Commands → Services → DAO → Database)

**Key Components**

- **ProviderService**: Provider CRUD, switching, backfill, sorting
- **McpService**: MCP server management, import/export, live file sync
- **ProxyService**: Local proxy mode with hot-switching and format conversion
- **SessionManager**: Conversation history browsing across all supported apps
- **ConfigService**: Config import/export, backup rotation
- **SpeedtestService**: API endpoint latency measurement

</details>

<details>
<summary><strong>Development Guide</strong></summary>

### Environment Requirements

- Node.js 18+
- pnpm 8+
- Rust 1.85+
- Tauri CLI 2.8+

### Development Commands

```bash
# Install dependencies
pnpm install

# Dev mode (hot reload)
pnpm dev

# Type check
pnpm typecheck

# Format code
pnpm format

# Check code format
pnpm format:check

# Run frontend unit tests
pnpm test:unit

# Run tests in watch mode (recommended for development)
pnpm test:unit:watch

# Build application
pnpm build

# Build debug version
pnpm tauri build --debug
```

### Rust Backend Development

```bash
cd src-tauri

# Format Rust code
cargo fmt

# Run clippy checks
cargo clippy

# Run backend tests
cargo test

# Run specific tests
cargo test test_name

# Run tests with test-hooks feature
cargo test --features test-hooks
```

### Testing Guide

**Frontend Testing**:

- Uses **vitest** as test framework
- Uses **MSW (Mock Service Worker)** to mock Tauri API calls
- Uses **@testing-library/react** for component testing

**Running Tests**:

```bash
# Run all tests
pnpm test:unit

# Watch mode (auto re-run)
pnpm test:unit:watch

# With coverage report
pnpm test:unit --coverage
```

### Tech Stack

**Frontend**: React 18 · TypeScript · Vite · TailwindCSS 3.4 · TanStack Query v5 · react-i18next · react-hook-form · zod · shadcn/ui · @dnd-kit

**Backend**: Tauri 2.8 · Rust · serde · tokio · thiserror · tauri-plugin-updater/process/dialog/store/log

**Testing**: vitest · MSW · @testing-library/react

</details>

<details>
<summary><strong>Project Structure</strong></summary>

```
├── src/                        # Frontend (React + TypeScript)
│   ├── components/
│   │   ├── providers/          # Provider management
│   │   ├── mcp/                # MCP panel
│   │   ├── prompts/            # Prompts management
│   │   ├── skills/             # Skills management
│   │   ├── sessions/           # Session Manager
│   │   ├── proxy/              # Proxy mode panel
│   │   ├── openclaw/           # OpenClaw config panels
│   │   ├── settings/           # Settings (Terminal/Backup/About)
│   │   ├── deeplink/           # Deep Link import
│   │   ├── env/                # Environment variable management
│   │   ├── universal/          # Cross-app configuration
│   │   ├── usage/              # Usage statistics
│   │   └── ui/                 # shadcn/ui component library
│   ├── hooks/                  # Custom hooks (business logic)
│   ├── lib/
│   │   ├── api/                # Tauri API wrapper (type-safe)
│   │   └── query/              # TanStack Query config
│   ├── locales/                # Translations (zh/en/ja)
│   ├── config/                 # Presets (providers/mcp)
│   └── types/                  # TypeScript definitions
├── src-tauri/                  # Backend (Rust)
│   └── src/
│       ├── commands/           # Tauri command layer (by domain)
│       ├── services/           # Business logic layer
│       ├── database/           # SQLite DAO layer
│       ├── proxy/              # Proxy module
│       ├── session_manager/    # Session management
│       ├── deeplink/           # Deep Link handling
│       └── mcp/                # MCP sync module
├── tests/                      # Frontend tests
└── assets/                     # Screenshots & partner resources
```

</details>

## Known Limitations

### Claude accounts need a browser re-login every few weeks

Switchy can capture an "Official" Claude provider's OAuth state and swap between multiple accounts on demand, including accounts you have not touched for days.

Two deadlines govern how long a captured account stays usable. The access token expires in about 8 hours and is renewed automatically. The refresh token behind it has its own expiry, typically one to several weeks out, and that deadline is anchored to when you originally signed in through the browser — renewing does not push it back. Once it passes, that account needs `claude /login` again; no local state management extends it.

### One login, two installs

Claude Code and Codex both rotate the refresh token on every renewal, and the server accepts each one only once. Two installs holding the same login — most commonly Windows and WSL — will therefore race, and the one that renews second is rejected and left signed out.

Switchy reconciles the two sides in the background, moving the surviving login to whichever side lost, so this heals on its own. It needs the mirror directory configured and reachable (one per tool, under Settings → Directories); while WSL is shut down, a rotation that happens on the Windows side cannot be propagated until it comes back. For Codex, an install that is on an API key rather than a ChatGPT login is left alone.

### Switching Codex accounts in an open session

A running Codex session reads its ChatGPT login once, at start, and refuses to reload a login that belongs to a different account. Switching the Official provider therefore changes the account for the next session, not the open one.

Turn on the proxy for Codex (Settings → Proxy → Local Proxy) and the open session switches too. Codex keeps its own login and its built-in provider is pointed at the local proxy; on every request the proxy replaces that login with the one of the Official provider currently selected. Codex stays in ChatGPT mode, so its model list and its `codex resume` history are unchanged. Sessions started before the proxy was turned on keep talking to OpenAI directly until they are restarted.

- **Rotation.** With automatic failover on, the failover queue is the pool of accounts. *Rotate accounts before the limit* (Settings → Proxy → Auto Failover → Codex) moves an account whose usage has reached the threshold to the back of the queue until its limit resets, and an account OpenAI refuses for usage is passed over until the reset it names. When every account is spent the request still goes out, so you see OpenAI's own message.
- **Logins stay usable.** The proxy renews a stored login when its access token runs out and hands the renewed login to Codex's own `auth.json` and to the copy restored when the proxy is turned off, under the same newest-valid-login-wins rules as [One login, two installs](#one-login-two-installs). A login OpenAI has rejected is not retried; sign in again with `codex login` while that provider is current.
- **Exit check.** Before a stored login is used, the proxy asks the edge in front of the service (`chatgpt.com`, or `api.anthropic.com` for Claude) where it sees this machine, over the route the request will take. If that is mainland China, or the question gets no answer, the request is held for up to 30 seconds and then refused; nothing is sent. Set Switchy's global outbound proxy if this machine reaches the internet through a local proxy port rather than a router or TUN path.
- A Codex install in WSL is not routed through the Windows proxy.

### Rotating Claude accounts through the proxy

Claude Code picks up a switch without a restart, so Official Claude accounts already swap by hand at any time. Rotating them automatically goes through the proxy: turn Local Proxy on for Claude (Settings → Proxy → Local Proxy). An Official account whose login has not been captured cannot be served, so Switchy refuses the takeover and says so; capture it from the provider card first. Claude Code sessions started while the proxy was on keep calling it after it is turned off; quit and reopen them.

With the proxy taken over for Claude, Claude Code keeps its own subscription sign-in and only its API address changes. The proxy presents the selected Official account's captured login on each request and patches the account id Claude Code writes into the request to match it. Rotation, renewal, the exit check and the login rules are the ones described for Codex above; Anthropic's rate-limit headers decide when an account is spent, and an account it refuses is passed over until the reset it names. Claude Code's own identity calls (`/api/oauth/*`) pass through with the login Claude Code sent, so it never learns another account's identity.

Switchy then sits in the request path and renews captured logins itself, using Claude Code's own client id against Anthropic's token endpoint. Whether that fits Anthropic's terms for subscription logins is yours to weigh before turning the proxy on for Claude.

### Keeping pooled accounts warm

A subscription's session window — Anthropic's five hours, ChatGPT's equivalent — opens on a real request and resets a fixed time later. An account nobody has used has no window running, so when rotation moves onto it the window starts from cold, and that account is the one holding the session with its reset furthest away.

*Keep accounts warm* (Settings → Proxy → Auto Failover → Claude or Codex) opens those windows in advance. Each Official account is looked at on the interval you set, and one request for a single token goes out to the ones whose window has lapsed. An account whose window is still running is skipped, and so is one at its limit, so the cost stays near one request per account per window. It runs while Switchy is running and needs neither the proxy nor rotation to be on; it is off by default, because this spends quota with nobody present.

Presenting the login renews it, so a warmed account is also a signed-in one — but only as far as its refresh token reaches. That deadline is set by the last sign-in in the browser and renewing does not push it back, so keep-warm cannot hold an account open indefinitely; when the refresh token expires the account needs `claude /login` or `codex login` while it is the current provider. Everything else is as it is for a real request: the exit check runs first, a rejected refresh token is not retried, and the quota the answer reports is recorded for rotation to use — which is the other thing keep-warm buys, since an account nobody has used reports nothing at all.

The model each request uses is the test model for that tool under Settings → Advanced → Model Test Config, which defaults to the cheapest one.

### What a Gemini or Kimi card can show

Usage badges and account switching both come from signing a tool in with an account. Claude Code, Codex and Kimi Code keep that login in a file Switchy stores with each provider, so switching providers switches accounts.

Gemini CLI is different. A Gemini provider holds the API key and endpoint, not the Google sign-in, so switching Gemini providers never changes which Google account is signed in. Gemini usage badges appear only while Gemini CLI is signed in with Google (`/auth` → Login with Google); on an API key there is nothing to show.

Kimi Code publishes no usage figures at all, so Kimi cards never show badges.

## Contributing

Issues and suggestions are welcome!

Before submitting PRs, please ensure:

- Pass type check: `pnpm typecheck`
- Pass format check: `pnpm format:check`
- Pass unit tests: `pnpm test:unit`

For new features, please open an issue for discussion before submitting a PR. PRs for features that are not a good fit for the project may be closed.

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=registered2nd/switchy&type=Date)](https://www.star-history.com/#registered2nd/switchy&Date)

## License

MIT © Jason Young
