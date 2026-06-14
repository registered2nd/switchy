# Fork Notes: Windows + WSL Claude Sync

This fork adds an optional second Claude configuration target so provider switches can be mirrored from the Windows Claude config directory to a WSL Claude config directory.

## What Changed

- Added `Claude Code Mirror Directory` in Settings > Advanced > Configuration Directory Override.
- When Claude live config is written, the app now also updates a second Claude config target if configured.
- Intended use case: keep Windows `~/.claude` and WSL `~/.claude` in sync from one Switchy instance.

## Important Behavior

This is a provider-field sync, not a full-file mirror.

That means:

- Stock Switchy writes the selected provider snapshot to one Claude config directory.
- This fork writes the full Claude snapshot to the primary Claude config directory.
- For the mirror target, this fork preserves existing machine-specific config and only updates provider-relevant Claude fields:
  - selected provider/auth env keys such as `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, and `ENABLE_TOOL_SEARCH`
  - `model`
  - `permissions`
  - `effortLevel`
- If the mirror target exists but cannot be parsed, the fork skips mirror sync for that write instead of falling back to a full-file overwrite.

## Consequence

If the WSL Claude config has Linux-specific hooks, status-line commands, plugin settings, or other local-only fields, they are preserved on the mirror target instead of being replaced by the Windows-side config.

This fork is therefore correct for:

- switching provider lanes on both sides at once
- keeping Windows-vs-WSL machine-specific customization separate

This fork is not correct for:

- forcing both sides to remain byte-for-byte identical across all Claude settings fields

## Current Semantics

After the mirror directory is configured and saved:

1. Switching Claude providers rewrites the Windows Claude config.
2. The configured WSL Claude directory is updated with only the provider-related Claude fields from the active Windows config.
3. WSL-specific hooks, status line, plugins, and other machine-local settings remain in place.
4. Switching back to another provider updates those same provider fields again on both sides.

This keeps both sides aligned on provider lane while allowing each side to keep its own local runtime wiring.

## Example Mirror Path

For the local machine used during development, the working WSL mirror path was:

`\\wsl$\Ubuntu-22.04\home\agentcode\.claude`

## Validation Notes

This fork was validated locally with:

- `pnpm typecheck`
- `cargo test --lib services::provider::live -- --nocapture`
- `pnpm build:renderer`

The Tauri packaging flow produced runnable debug artifacts, but the final package command reported a signing-key error because the environment had a public key without a matching private signing key. That does not block local execution of the built app.
