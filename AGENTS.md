# Switchy

- Use `README.md` for product/build/deploy guidance, `DECISION_LOG.md` for durable choices, and `CHANGELOG.md` for released history.

## Workstate-native handoff

Repository key: `switchy`. Resume with `python -m workstate.cli handoff-show switchy`. On an explicit handoff trigger, close from one durable manifest with `python -m workstate.cli --actor <actor> --run <run> handoff-close switchy <durable_manifest.json>`.
