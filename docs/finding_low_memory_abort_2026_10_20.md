# Finding: Switchy aborts when Windows runs out of commit memory

**Status:** cause confirmed from the crash dump. No code change made yet; options ranked below.
**Observed:** 2026-10-20 05:10:37 (Pacific), workstation, Switchy 1.0.39, Windows 11 build 26200.

## What happened

At 05:10:37 the Resource-Exhaustion-Detector logged "computer is low on virtual memory" and, in the same second, switchy.exe (pid 66532, running since 2026-10-19 11:09) ended with exception `0xc0000409` at offset `0x321c11`, WER bucket `BEX64`, fail-fast code 7. The log stops mid-traffic at 05:10:35 and `crash.log` was not written. Switchy stayed down until a manual start at 06:24:53, so every tool routed through port 15721 had no endpoint for 74 minutes. On that start Switchy logged "Previous run exited abnormally (takeover leftovers found)", restored the live configs and took them over again within a second.

## Cause: a refused allocation while forwarding a Claude request

Windows kept a minidump because the machine has the global WER `LocalDumps` key: `%LOCALAPPDATA%\CrashDumps\switchy.exe.66532.dmp`. The release binary has no symbols, so the frames were named by unwinding the faulting thread with the binary's own unwind table and reading the source locations and strings each function references.

The faulting thread is a `tokio-rt-worker`. From the abort upwards:

| Frame | Function | Identified by |
|---|---|---|
| `0x321be0` | std `rust_oom`: calls the OOM hook, then `__fastfail(7)` | the `int 0x29` with `ecx = 7` at the fault offset |
| `0x334c37` (called) | std default OOM hook | references "memory allocation of {} bytes failed" |
| `0xebec05`, `0xebec1f` | `handle_alloc_error`, `raw_vec::handle_error` | the allocation-error branch, not the capacity-overflow one |
| `0xebea50`, `0x4d52f` | growing a `Vec<u8>` | |
| `0xc55eb1` … `0x3bbc7e` | serde_json writing escaped strings, arrays and objects | serde_json's escape table and `ser.rs` locations |
| `0x5170fa` | the forwarder's send function: `serde_json::to_vec(&filtered_body)` | "Failed to serialize request body" (`forwarder.rs:1430`) |
| `0x52cd2b`, `0x5229a6` | `forward`, `forward_with_retry` | `forwarder.rs` locations |
| `0xb15e33` | `handle_messages` | `handlers.rs` locations |

The refused request was 1,228,800 bytes (align 1): the request buffer held 612,464 bytes in a 614,400-byte allocation, an 8,876-byte string had to be appended, and the doubled allocation failed. Nothing about the request was unusual. The dump's system counters show commit at its ceiling: peak commitment 255.38 GB equal to the 255.38 GB limit, with 230.1 GB still committed when the dump was written 36 seconds later.

Rust does not panic on a refused allocation. It prints "memory allocation of N bytes failed" to stderr, which a windowed app does not have, and aborts. The panic hook therefore never runs, whatever the `panic` profile setting, and `crash.log` stays silent for this whole class of failure.

## Switchy's own footprint

At the crash, after 18 hours: 460 MB private commit, 430 MB working set, peaks of 953 MB and 899 MB.

The current run (started 06:24:53), sampled every 10–30 seconds from 07:11 to 08:03: private commit between 75 and 185 MB when idle, rising to 260–534 MB within seconds of a burst of requests and falling back within a minute; peak 600 MB. The idle level did not rise across the 52 minutes (175 MB at 07:11, 94 MB at 07:50, 128 MB at 08:00), so the memory comes and goes with traffic rather than accumulating. CPU averaged about a sixth of one core. Its six WebView2 processes add about 224 MB private.

The swings come from how each request body is handled. For a body of N bytes, the Claude path holds at the moment of sending:

- the raw body bytes and the parsed JSON tree (`handlers.rs`, `handle_messages`);
- a clone of that tree passed to `forward_with_retry` while the handler keeps its own (`handlers.rs`);
- a clone per provider attempt (`forwarder.rs`, `provider_body`);
- a clone for model mapping (`forwarder.rs`, `apply_model_mapping(body.clone())`);
- the serialized body, grown by doubling (`forwarder.rs:1430`, the allocation that failed);
- a second full serialization made only for a debug log line, run on every request even at the default `info` level (`forwarder.rs:1448`).

Streamed responses add the usage collector, which keeps every parsed SSE event until the stream ends (`response_processor.rs`, `SseUsageCollector`), although usage needs only a few of them. Logs go to disk, not memory. The long-lived maps are keyed by account, provider or Codex window, so none grows with traffic.

## What filled commit

The commit limit is 63.4 GB of RAM plus a 192 GB system-managed page file, already at its ceiling of three times RAM. Only 17.8 GB of the page file is in use (peak 40.6 GB): most of the commit is charged and never touched. At 07:10, 189.5 GB was committed, about 70 GB of it by MCP servers that every Claude Code session starts its own copy of: about 360 Playwright MCP node processes (32 GB), 74 windows-mcp Python processes plus 44 uv launchers (37 GB), and 76 AdsPower MCP node processes (6 GB). 45 claude.exe processes held another 33 GB. Commit therefore scales with the number of open sessions, and Switchy, which allocates on every request, is a likely process to be holding a refused allocation when the ceiling is reached. Freeing commit is the `system_reclaim` skill's ground.

## Options, ranked

1. **Register for Windows' crash restart.** At startup on Windows, call `RegisterApplicationRestart` with `--autostart` and the no-patch and no-reboot flags, so Windows Error Reporting relaunches Switchy into the tray after a crash or a hang. Tested 2026-10-20 with a stand-alone program that registered the same way and then died of a refused allocation (`0xc0000409`, `BEX64`): WER relaunched it 7 seconds after the crash with the registered argument, with no dialog. About ten lines of code. WER restarts only a process that ran for at least 60 seconds, which guards against crash loops, and tries once. It does not fire on a normal quit, `taskkill` or the installer's stop, so it leaves the reinstall procedure alone. The relaunch waits until WER has written its report; on 2026-10-20 the dump took 36 seconds under memory pressure, by which time commit had fallen to 230 GB.
2. **An event-triggered scheduled task.** A task triggered by Application event 1000 for `switchy.exe` starts `switchy.exe --autostart` and retries every minute if the launch fails. It covers the same crashes and keeps retrying, but it is a second mechanism outside the app that Switchy would register and remove alongside start-at-login, and it has no crash-loop guard. Worth adding only if option 1's single attempt proves too little.
3. **Ride out a brief exhaustion.** A global allocator wrapper that, when an allocation is refused, waits and retries for a few seconds and writes a fixed line (size, time) to a pre-opened `crash.log` before letting the abort go ahead. Commit fell by 25 GB within 36 seconds of this crash, so a short wait might have avoided it entirely; that is not verified. About forty lines of unsafe code that must not itself allocate; while memory is exhausted, the threads involved stall.
4. **Record the cause on the next start.** Switchy already detects the abnormal exit from takeover leftovers. Writing the Windows crash event (exception code, time) and the dump path into `crash.log` on that start would make the next occurrence explain itself. Diagnostic only.
5. **Hold fewer copies of each request.** Drop the debug-only serialization, clone the parsed body only when a provider changes it, and keep only usage-bearing SSE events. This lowers Switchy's peaks but cannot prevent the crash: at the machine's limit any allocation fails, however small.
6. **A supervisor process.** Restarts on any exit and retries indefinitely, but needs its own start-at-login entry and changes to single-instance handling, the updater and the stop step of the reinstall procedure, which a supervisor would race. Highest cost and risk for little beyond options 1 and 2.
7. **Keeping routed tools working while Switchy is down.** Nothing practical: running Claude Code and Codex sessions keep calling `127.0.0.1:15721`, restoring direct configs reaches only new sessions, and a stand-in listener could not serve the pooled logins. Shortening the outage is what helps.
