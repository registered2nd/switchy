# Requirements — Snapshot OAuth Refresh (on-demand, user-initiated)

> **ABANDONED 2026-04-20.** This feature is ToS-prohibited under Anthropic's April 2026 "Authentication and credential use" policy. See `DECISION_LOG.md` 2026-04-20. Document retained as a historical artifact of the scoping work that was done before the policy finding; do not resume. Design and tasks stages were never written.


## Purpose

Let a Switchy user get a fresh subscription-quota reading for a captured-but-not-current Official Claude provider, without having to switch into it and run `claude /login`. The refresh fires **only** when the user explicitly clicks the refresh button on that provider's card — never on card mount, never on a timer.

This unblocks the stuck "Session expired" badge described in `BACKLOG.md` #5, while keeping Switchy's OAuth-endpoint call pattern indistinguishable from normal Claude Code use (one refresh per user gesture, keyed to a specific captured account).

## Scope boundary

### This spec owns

- A new code path in the captured-snapshot quota flow that, on **user-initiated** refresh of a captured Official Claude card with an expired `access_token` and a present `refresh_token`, performs an OAuth `grant_type=refresh_token` exchange and atomically rewrites the snapshot file before the usage-API call proceeds.
- The UI gating that distinguishes "mount-time query" from "user-initiated refresh" so the refresh endpoint is never hit on passive render.
- A settings-level feature flag (off by default) controlling whether the OAuth refresh path runs at all. When off, the card falls back to today's Expired behavior.
- Atomic write-back of the refreshed `{accessToken, refreshToken, expiresAt}` into the captured snapshot, reusing the existing `claude_account::store::atomic_write` helper.
- Structured logging of every refresh attempt (provider id, UUID, outcome — success / OAuth failure / HTTP failure), emitted to `switchy.log` at INFO for success and WARN for failure.

### This spec does NOT own

- On-mount refresh (that is the explicitly-rejected 5a variant — see "Non-goals").
- Periodic background refresh loops (`BACKLOG.md` #6).
- Refresh for the *current* account — the live `get_subscription_quota` path continues to rely on Claude Code's own refresh of `~/.claude/.credentials.json`. Switchy does not call the OAuth endpoint for the live card.
- Codex / Gemini / OpenCode / OpenClaw providers. Claude Official only.
- Changes to the existing `switch-away` capture flow (shipped in 1.0.0, commit `08f19e2c`). That flow writes the snapshot; this flow reads it and conditionally refreshes it.
- Changes to the Claude Code `/login` fallback path. If OAuth refresh fails or is disabled, the user still has today's `claude /login` escape.

## User stories

### US-1: Refresh a stale captured account without switching into it

As a Switchy user with two captured Official Claude accounts, when I look at the second card an hour after capturing it and see "Session expired," I want to click the refresh button once and get fresh quota numbers — without being forced to switch into that account and re-authenticate.

**Acceptance criteria:**
- AC-1.1 A captured Official Claude card whose snapshot has an expired `access_token` but a present non-empty `refresh_token` displays the "Session expired" state exactly as it does today on first render. The refresh button is present and enabled.
- AC-1.2 When the user clicks the refresh button, Switchy performs exactly one POST to `https://console.anthropic.com/v1/oauth/token` with body `{grant_type: "refresh_token", refresh_token: <from snapshot>, client_id: <Claude Code's client_id, to be captured during design>}`.
- AC-1.3 On a 2xx response containing `{accessToken, refreshToken, expiresAt}`, the captured snapshot file is atomically rewritten with the new values (path: Switchy's internal account-snapshot path for this provider — exact construction in design). No other fields in the snapshot are modified.
- AC-1.4 Immediately after write-back, the existing usage-API query runs against the fresh `access_token` and the card transitions from the Expired state to a normal quota display within the same user gesture (no second click required).
- AC-1.5 A log line at INFO level is written to `switchy.log`: `[snapshot_oauth_refresh] provider={uuid} refresh ok, expires_at={new_expires_at}`.

### US-2: Refresh failures never break the Expired fallback

As a Switchy user whose refresh_token has been revoked or whose network is down, when I click refresh, I want to see the same "Session expired" state I'd see today — not a broken UI or a silent hang.

**Acceptance criteria:**
- AC-2.1 On any non-2xx response from the OAuth endpoint (including 401, 403, 429, 5xx), Switchy does **not** modify the snapshot file. The card remains in the Expired state. The refresh button becomes re-enabled.
- AC-2.2 On network failure (timeout, DNS failure, TLS failure), same behavior as AC-2.1.
- AC-2.3 A log line at WARN level is written to `switchy.log`: `[snapshot_oauth_refresh] provider={uuid} refresh failed: {reason}` where `{reason}` distinguishes HTTP status codes from transport failures.
- AC-2.4 The user is never shown a raw HTTP error string in the UI. The Expired badge's existing text and recovery-hint copy are unchanged.
- AC-2.5 The refresh attempt has a hard timeout of 10 seconds. After that the request is aborted and the failure path runs.

### US-3: Feature is off by default and respects the flag

As a Switchy user who does not want the app to hit Anthropic's OAuth endpoint on my behalf, I want snapshot OAuth refresh to be off unless I explicitly turn it on in settings.

**Acceptance criteria:**
- AC-3.1 A new boolean setting `claude.snapshotOauthRefreshEnabled` exists in the settings schema, defaults to `false`, and is persisted through the existing settings mechanism.
- AC-3.2 When the flag is `false`, clicking the refresh button on an Expired captured card performs zero OAuth endpoint calls. The behavior is identical to today (the button re-queries the snapshot, finds it still expired, and the Expired state persists).
- AC-3.3 When the flag is `true`, the behavior defined in US-1 and US-2 applies.
- AC-3.4 The settings UI surface for this flag includes copy that explicitly names the reliability risk: that Anthropic may rate-limit or block third-party OAuth calls, and that enabling the flag may result in token invalidation requiring a `claude /login`.
- AC-3.5 Toggling the flag takes effect on the next refresh click without requiring an app restart.

### US-4: On-mount query path is untouched

As a user who has multiple captured cards on screen, when the app renders them, I want Switchy to make at most one quota call per card (to Anthropic's usage endpoint with the *existing* access_token) — never an OAuth refresh call.

**Acceptance criteria:**
- AC-4.1 The React Query hook `useSubscriptionQuotaForProvider` (`src/lib/query/subscription.ts:23`) continues to fire on card mount and on React Query cache invalidation. It MUST NOT trigger the OAuth refresh path, even when the snapshot is Expired.
- AC-4.2 The OAuth refresh path is reachable from exactly one UI entry point: the refresh button's `onClick` handler on `SubscriptionQuotaFooter` (`src/components/SubscriptionQuotaFooter.tsx:129–136` and parallel branches at `:152–159`, `:221–231`, `:260–267`). The mount-time query and the button-click query use distinct backend commands so the distinction cannot be accidentally collapsed by a React Query refactor.
- AC-4.3 Verification: with the flag on and a captured Expired card, loading the main screen produces zero log lines matching `[snapshot_oauth_refresh]`. Only after a deliberate refresh-button click does such a line appear.

## Non-goals (explicitly rejected)

- **On-mount refresh (the "5a" variant).** Triggering OAuth refresh whenever a captured card with an expired access_token renders. Rejected because the resulting call pattern (N refreshes per app launch, one per captured Expired card, zero user intent) is the pattern Anthropic's endpoint hardening targets — the same pattern sub2api operators report getting 429/500s on.
- **Periodic background refresh loop** (`BACKLOG.md` #6). Not built until the on-demand path has proven stable across several weeks of real use. Explicitly out of scope here.
- **Refresh for the live/current account.** Claude Code refreshes its own `~/.claude/.credentials.json` — Switchy does not need to and will not.
- **Retry on failure.** A failed refresh produces the Expired fallback; no automatic retry. The user can click the button again themselves.
- **Caching of the new token beyond the snapshot file.** The snapshot file is the sole authority; no in-memory cache keyed to refresh_token.

## Upstream dependencies

| Source | Purpose | Specific shape Switchy depends on |
|---|---|---|
| Captured snapshot file at Switchy's internal account-snapshot path | Captured OAuth snapshot written by `sync_outgoing_snapshot` (`src-tauri/src/services/claude_account/mod.rs`, shipped 1.0.0 `08f19e2c`) | JSON object containing `claudeAiOauth: { accessToken, refreshToken, expiresAt (Unix ms), scopes[], subscriptionType, rateLimitTier }` plus a sibling `mcpOAuth` object (untouched by this spec). **Confirmed by inspection of real 1.0.0 snapshots on 2026-04-19** — O-2 resolved; `refreshToken` is preserved. The on-disk path is constructed via Switchy's internal app-config-dir helpers (Rust side; `get_app_config_dir()` + `accounts/{providerId}/credentials.json`), not shell `~` expansion; design owns the exact construction. AC-1.3's "no other fields modified" rule applies to every field above *and* to `mcpOAuth`. |
| Anthropic OAuth refresh endpoint | Exchange refresh_token for a new token pair | POST `https://console.anthropic.com/v1/oauth/token`, `Content-Type: application/json`, body `{grant_type, refresh_token, client_id}`. 2xx returns `{accessToken, refreshToken, expiresAt}`. Shape assumption to be confirmed during design against Claude Code's own refresh call (e.g., by packet-capturing one). **Open question O-1.** |
| `claude_account::store::atomic_write` | Atomic snapshot write-back | Already in use for snapshot capture. Design reuses it verbatim. |
| Existing usage-API query in `services::subscription::get_claude_quota_for_provider` | After refresh, run the existing quota query against the new access_token | `src-tauri/src/services/subscription.rs:200–237`. Reads the snapshot file fresh on each call. Design must ensure the refresh-and-requery sequence re-reads the snapshot after write-back so it picks up the new token. |
| Switchy settings schema | Persist the feature flag `claude.snapshotOauthRefreshEnabled` | Pattern for adding a new boolean flag (file path, serde shape, default handling) must be identified during design. The flag must round-trip through the existing settings load/save path without schema-migration scaffolding. |

## Downstream consumers

- `SubscriptionQuotaFooter` (`src/components/SubscriptionQuotaFooter.tsx`) — receives the refresh-and-query result and re-renders. No shape change to the `SubscriptionQuota` type returned; the feature changes *when* a fresh query succeeds, not what a success looks like.
- `ProviderCard` (`src/components/providers/ProviderCard.tsx`) — unchanged. Still passes `providerId` only for captured cards.

## Constraints

- **Best-effort.** A failed OAuth refresh must never prevent the card from rendering, must never propagate an exception to React Query's error boundary, and must never block the switch flow (the switch path does not call this refresh path at all — but stated here to preserve the invariant the 1.0.0 sync-away code ships with).
- **One call per click.** No retries, no parallel calls from simultaneous card renders. If the user clicks refresh twice in quick succession on the same card, the second click is a no-op while the first is in flight (enforce via the existing React Query `isFetching` gate, already reflected in the button's `disabled={loading}` prop).
- **Client fingerprint.** The `client_id` sent to the OAuth endpoint must match the one Claude Code uses when refreshing its own credentials. Picking a different `client_id` is an immediate token invalidation signal. Design owns capturing this value from a live Claude Code refresh.
- **Refresh_token rotation.** The OAuth response may include a new `refreshToken` different from the one we sent. Switchy must write back the new `refreshToken` — failing to do so will break the *next* refresh for this account.
- **No header/UA spoofing beyond what Claude Code does.** If Claude Code sends specific User-Agent or other headers on its refresh, Switchy matches those. Switchy does not invent headers to disguise itself as Claude Code beyond what Claude Code itself sends. Design stage owns capturing the exact request shape.
- **Off by default.** AC-3.1 is load-bearing: users should not be surprised by OAuth calls happening on their behalf until they opt in.

## Accepted tradeoffs

- **Token invalidation is a possible outcome.** If Anthropic fingerprints non-official clients on the OAuth endpoint and invalidates our refresh_token, the captured account's snapshot becomes un-refreshable and the user must `claude /login` into that account and re-capture. This is stated in the settings-UI copy (AC-3.4). We do not attempt to mitigate this risk beyond matching Claude Code's call shape as closely as design stage can capture.
- **No equivalent for Codex/Gemini/OpenCode/OpenClaw.** Those providers have their own OAuth shapes (or don't use OAuth at all). This spec intentionally ships Claude-only; other providers' captured quota-pills remain at today's Expired behavior.

## Open questions (for design stage)

The first two are **blocking gates** — design cannot be approved without resolving them, because they determine whether the spec is even feasible or safe to implement. The rest are normal design-stage questions.

- **O-1 (BLOCKING): Exact OAuth request shape used by Claude Code.** Required to match the call fingerprint. Resolution: during design, capture one real Claude Code refresh-token exchange (via mitmproxy or equivalent) and document the full request — method, URL path, headers (User-Agent, auth, content-type), body field names, encoding. Design doc must include an "OAuth Request Shape" section with that captured reference. Without it, the feature risks token invalidation on first call.
- **O-2 (RESOLVED 2026-04-19):** `refreshToken` is present in 1.0.0 captured snapshots. Confirmed against `~/.switchy/accounts/{4b88712c...,b0d59e50...}/credentials.json` — both carry valid `sk-ant-ort01-*` tokens under `claudeAiOauth.refreshToken`. Full shape documented in the Upstream dependencies table. No change to `sync_outgoing_snapshot` required.
- **O-3: What happens on a `refreshToken`-rotation failure mid-write?** If we POST and get a 2xx with a new token, then the atomic write fails (disk full, permission error), we have minted a new token pair that the server has recorded as issued but we've lost locally. Design must decide: is this tolerable (user re-logs in), or does design need a two-phase-commit shape?
- **O-4: How does the settings UI surface land?** Settings copy in AC-3.4 is load-bearing. Design stage owns the exact wording and placement, and must identify which existing settings pane the toggle belongs in.
- **O-5: Distinct-command mechanism for AC-4.2.** The constraint is that mount-path and refresh-path cannot be accidentally collapsed. Design must decide between (a) two distinct Tauri commands (e.g., `get_subscription_quota_for_provider` kept as-is for mount, plus new `refresh_and_query_snapshot_quota` for the button click), or (b) a single command with an explicit `allow_oauth_refresh: bool` parameter that only the button handler sets to `true`. Either satisfies AC-4.2; design picks one with rationale.

## Verification summary

A feature-complete build passes the following checks:

| Check | How to verify |
|---|---|
| AC-1.1 — Expired card renders as today | Capture an account, wait 1h+, observe the Expired badge exists with refresh button enabled |
| AC-1.2, AC-1.4 — One refresh call, one state transition | Flag on, click refresh on Expired card, observe: one OAuth POST in network capture, one write to snapshot file, card transitions to quota display |
| AC-1.3 — Only token fields written | Diff snapshot file before/after; only `accessToken`, `refreshToken`, `expiresAt` under `claudeAiOauth` change |
| AC-1.5, AC-2.3 — Log lines | Grep `switchy.log` for `[snapshot_oauth_refresh]` after each scenario |
| AC-2.1, AC-2.2 — Failure preserves Expired | Point OAuth URL at a test server returning 401/500/timeout; confirm snapshot unchanged, Expired state persists |
| AC-2.5 — 10s timeout | Test server holds connection open; abort fires at 10s |
| AC-3.1, AC-3.2 — Flag off is no-op | Flag off, click refresh on Expired card, confirm zero OAuth POSTs in network capture |
| AC-3.3, AC-3.5 — Flag on takes effect immediately | Toggle flag without restart; next click follows US-1 path |
| AC-4.1, AC-4.2 — Mount path untouched | Flag on, Expired captured card, load main screen; zero `[snapshot_oauth_refresh]` log lines |
| AC-4.3 — One entry point | grep `src/` for callers of the new backend command; exactly one call site in the refresh-button handler path |
