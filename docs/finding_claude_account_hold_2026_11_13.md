# Finding: what Anthropic saw before the pooled Claude account user6 was held (2026-11-13)

Status: provisional. Anthropic names no cause ("We can't share the specifics"); nothing below is confirmed as the trigger. Workstate SW319.

## The event

- The account (Switchy card `8d160b0e`, another user's account, browser login on another user's PC) carried no real traffic from 2026-11-09 08:29 UTC, when rotation moved off it at its 95% limit. Keep-warm sent its one-token request at 2026-11-13 07:00 UTC and got a 200.
- 2026-11-13 19:46 UTC: a manual switch moved the pool onto it. Between 19:47:00 and 19:48:55 about 60 requests succeeded, mostly `claude-opus-5-5`, a few `claude-fable-5-2`, at most 12 at once.
- 19:48:56: the first 403, `permission_error`, `error_code: "verification_required"`, `capability: "avocado"`, with a link to `portal.anthropic.com/verify`. For a few seconds 403s and 200s interleaved, then only 403s. No other pooled account has ever received this error.
- 20:04: `OAuth access token has been revoked`. After the verification the account is on hold; claude.ai shows the review form at `claude.ai/restricted`, asking how the account was used **on Nov 13**. Reviews take about 10 days, and decisions are final.

How observed: Switchy's `proxy_request_logs` and `account_switches` (a read-only copy of `~/.switchy/switchy.db`), `~/.switchy/logs/switchy.log`, and a screenshot of another user's PC.

## What Anthropic could see from Switchy

| Signal | What went out |
|---|---|
| Exit | The workstation's network route, OVH Hillsboro `203.0.113.10` (datacenter), because the card has no proxy of its own. Minutes earlier the same conversations ran on user5 from DMIT Los Angeles `203.0.113.10`. a third user's traffic leaves from IPFoxy London `203.0.113.10`. |
| Device id | Claude Code's install id (`userID` in `~/.claude.json`), unchanged. The Windows install's id has gone out on all five pooled accounts; WSL's install sends its own. |
| Session id | Unchanged. A conversation rotated onto another account keeps its session id there, in `metadata.user_id` and in the `X-Claude-Code-Session-Id` header. |
| Account id | Patched to the serving account (`patch_account_uuid`). |
| Telemetry | Claude Code's own calls (`api.anthropic.com` event logging, Datadog `http-intake.logs.us5.datadoghq.com`) go through the CONNECT tunnel under the CLI's live login, carrying the real device id. |
| Sources | One machine. The proxy listens on `127.0.0.1` only; every connection comes from the workstation's Windows or WSL. |
| Volume | Up to 24,193 requests and 6.5 billion tokens (mostly cache reads) a day on this account (2026-11-08); peak 31 requests in flight (2026-10-26). |
| Keep-warm | A one-token "hi" claiming `user-agent: claude-cli/2.1.2 (external, cli)`, with no `metadata.user_id` and no Claude Code system prompt or client-library headers. It is sent to every pooled account whose window is not running. |

## How other pooling tools present an account

Read from each project's source at its 2026-11 head; shallow clones were in this session's scratchpad.

| Tool | Device id | Session id | Account id | Conversation stays on one account | Per account |
|---|---|---|---|---|---|
| Switchy | real, the same on every account | real, follows the conversation | patched | no | exit only if the card has a proxy |
| TeamClaude | real | real | patched | optional pins (off by default) | — |
| cc-switch upstream | does not pool Claude subscriptions; it refuses an official account behind its gateway, citing ban risk | | | | |
| claude-relay-service | real (optional fixed one) | hashed per account | patched | yes, 1 h | stored client-library headers, proxy |
| sub2api | random, made and kept per account | SHA256(account::session) as a UUID; header synced; optional mask | patched | yes, 1 h | user-agent, client-library headers, proxy, concurrency cap; drops `/api/event_logging/batch` |
| better-ccflare | real | real | not patched | yes, 5 h | — |

None of these projects says Anthropic checks device or session ids; the relays hide them as a precaution. Their users who report bans mostly blame shared or datacenter exits, volume beyond one person, mixed client versions on one account, and payment or region.

## Outside context

- 2026-11-12: Anthropic's new Usage Policy and Supported Regions Policy took effect. The AUP bars services that "route requests through consumer subscriptions or misrepresent the product or client being used", and multiple accounts used to avoid detection. The Consumer Terms (2025-10-08) bar making an account available to anyone else.
- 2026-11-11 to 13: users on V2EX, linux.do and GitHub report a wave of `verification_required` prompts and holds, including on accounts that were not pooled (e.g. [claude-mem #5251](https://github.com/thedotmack/claude-mem/issues/5251), [v2ex t/1256452](https://www.v2ex.com/t/1256452)).

## Candidate triggers, by evidence

1. **The policy sweep that started 2026-11-12.** It matches the timing best. The account was idle from 2026-11-09 and was checked within two minutes of its first heavy use after the policy took effect.
2. **A sudden change of profile.** The pool's whole load arrived on an idle account at once, from a datacenter exit unrelated to the account's own sign-in location. The load at the moment (12 in flight) was below the account's own past peak (31), so volume alone does not explain the timing.
3. **Cross-account identifiers.** The same device id is on five accounts, and session ids moved from user5 to this account minutes before the check. This is plausible, not shown. SW320 (installed 2026-11-13 21:26 UTC) gives each account its own device and session ids and turns Claude Code's telemetry off under takeover.
4. **Keep-warm's client identity.** A stale version string and a bare request body. Every pooled account sends it, so it is unlikely to be the trigger for this one; it is a standing risk.

## Verification path and revisit trigger

- The review outcome (about 10 days) and any email from Anthropic naming a reason.
- Whether another pooled account is checked or held, and what it carried at the moment: whether it had rotated in under load, its exit, and whether SW320 was live.
- Revisit when an account is held after SW320 is installed: if holds continue with distinct device and session ids per account, item 3 loses weight.
