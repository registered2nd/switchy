# Switchy

- Use `README.md` for product/build/deploy guidance, `DECISION_LOG.md` for durable choices, and `CHANGELOG.md` for released history.
- When reinstalling Switchy, run stop, silent install (`/S`), relaunch (`Start-Process "$env:LOCALAPPDATA\Switchy\switchy.exe"`, not through Explorer) and the wait for port 15721 as one command with nothing in between; run any other step before or after it. Never stop Switchy on its own. If that command is blocked, split off the other steps, never the four.

## Workstate-native handoff

Repository key: `switchy`. Resume with `python -m workstate.cli handoff-show switchy`. On an explicit handoff trigger, close from one durable manifest with `python -m workstate.cli --actor <actor> --run <run> handoff-close switchy <durable_manifest.json>`.
