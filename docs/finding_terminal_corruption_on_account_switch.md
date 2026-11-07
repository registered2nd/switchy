# Finding: switching Claude accounts corrupts Claude Code terminals in Cursor

**Status:** open, cause not established. Recorded so the ruled-out ground is not re-walked.
**Observed:** 2026-09-03 to 2026-09-06, Windows 11, Cursor 3.7.19, Claude Code 2.1.252 / 2.1.259 / 2.1.263.

## Symptom

Switching Claude accounts in Switchy, from the tray, visibly corrupts running Claude Code
sessions in Cursor's integrated terminal. Three shapes seen:

- Committed transcript lines re-emitted and interleaved with the current frame.
- Tables rendering only their first two rows; text drawn over text.
- A fullscreen session's transcript region going blank while the input box and footer survive.

Reloading the Cursor window does not fix it. Quitting Claude Code and resuming fixes it every
time. It is intermittent, and on 2026-09-06 it hit every terminal in the window at once rather
than only the focused one.

## Ruled out, with evidence

Each of these was tested directly on this machine, not reasoned about.

1. **The settings write.** `settings.json` was rewritten four times in a live session using both
   the old delete-then-rename sequence and the rename-only replacement. Claude Code's watcher
   logged exactly one reload each time and nothing corrupted.

2. **The content of the write.** A full account-shaped write was performed with no window or
   focus activity at all: account B's settings written, then account A's written back, a real
   503-byte difference each way. No corruption.

3. **Settings content reaching a running session.** A running session does not re-read model or
   renderer from `settings.json`. 568 transcript rows stayed on one model across a switch that
   wrote a different one, and a session stayed on the classic renderer the whole time
   `settings.json` said `tui: fullscreen`. Only fast mode and the advisor model are re-applied
   live.

4. **Credentials and the config JSON.** Identical rewrites of `.credentials.json`, singly and as
   a burst of six one second apart, plus swap-shaped account-state toggles in `~/.claude.json`
   over the same allowlist a swap uses, produced no corruption.

5. **Claude Code's version.** Seen on 2.1.252, 2.1.259 and 2.1.263. Terminal focus reporting is
   enabled in builds up to 2.1.259 and absent from 2.1.260 onward; corruption occurred on both
   sides of that boundary.

6. **Terminal focus.** The focus-triggered proactive repaint only runs in builds that ask the
   terminal to report focus. Corruption happened on builds that never ask.

7. **A resize or panel re-layout.** During an instrumented switch, three sessions' terminal
   buffers stayed at 298x36 across the whole event. Nothing resized.

8. **Anything on the machine changing.** Cursor 3.7.19 built 2026-06-07, no update since.
   Terminal GPU acceleration is off. Last Windows updates 2026-08-12/13, GPU drivers from
   January and June.

9. **Switchy's write path being unusual.** Upstream `cc-switch` writes the same files the same
   way for a much larger Windows user base and its tracker has no report of terminal corruption,
   searched in English and Chinese.

## What a switch demonstrably does reach every session with

An account-change notification. During one instrumented switch, all three monitored sessions
printed the same line within four seconds of each other:

> Remote Control disconnected — signed-in claude.ai account or organization changed on this
> machine — run /remote-control to start a session for the current account

This is emitted by Claude Code, not Switchy, and it has always accompanied an account change.
Rendering it forces a repaint in every running session simultaneously.

## Leading hypothesis, unproven

The corruption is that forced repaint landing on a session that is actively drawing. It fits
every property: window-wide fanout, intermittence, worst on busy or tall-transcript panes,
unaffected by a Cursor reload because the buffer is already wrong, and cured by quit-and-resume
because that redraws from scratch.

The one fully instrumented switch produced no corruption, and every session was idle at that
moment. That is consistent with the hypothesis but is a single negative result, not support.

## How to test it next

Switch while at least one session is mid-output with a tall transcript, and compare against a
switch with every session idle. If corruption tracks how busy the sessions are, rather than
which account is selected or which file changed, the hypothesis holds. If a busy session
survives a switch cleanly, the hypothesis is dead too.

## Instrumentation caveat

Reading another session's screen by attaching to its console is not passive. During one run it
leaked a Windows console error line into a live session's rendered output. A future probe should
capture from outside the console, or accept that it perturbs the thing it measures.

## Secondary finding: the two Official accounts' stored settings have drifted apart

Unrelated to the corruption, but real and worth knowing.

| Account | Bytes written to `settings.json` on switch |
|---|---|
| A | 3572 |
| B | 4075 |

Account B's stored snapshot carries an OpenTelemetry block with telemetry enabled and a local
collector endpoint, the fullscreen renderer, a different model, per-model effort settings and a
cleanup period. Account A carries none of it. Every switch therefore swaps roughly half a
kilobyte of unrelated configuration in and out of the live file.

This happens because switching away backfills whatever was live into the outgoing account, so
the two have been absorbing each other's interface and telemetry state over time. An account
swap is meant to move credentials and identity.
