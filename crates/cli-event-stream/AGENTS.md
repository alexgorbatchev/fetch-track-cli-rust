---
created_on: 2026-10-14 09:20
last_modified: 2026-10-14 22:40
status: current
---

# cli-event-stream development

This internal Cargo workspace package belongs to `fetch-track-cli` and inherits `../../AGENTS.md` and the migration parent's Rust instructions. Run local recipes from this package directory.

## Commands

- `just build`: build all targets.
- `just test`: run transport behavioral tests.
- `just fmt`: format this package only.
- `just check`: formatting, strict Clippy and at least 90% line coverage.
- Keep evidence and fixtures in `.tmp/`.

## Contracts

- Keep event schemas in consumers; accept native `serde::Serialize` records directly.
- Serialize each complete record while holding the shared writer mutex. Propagate connection, serialization, flush and write failures.
- Duplicate caller descriptors with close-on-exec; retain caller ownership and restore original open-file flags when the final shared writer drops. Use readiness-driven nonblocking I/O for pipes and sockets; regular files use asynchronous file I/O.
- Document exclusive descriptor use: callers must not concurrently write or reconfigure a selected descriptor while a reporter owns its duplicate. Cancellation can leave a partial final record; it does not establish delivery or rewind a stream.
- Test full-pipe cancellation in a bounded subprocess so runtime teardown regressions cannot hang the suite.
- Any runtime behavior change requires corresponding behavioral tests and at least 90% line coverage; `scripts/` is excluded. Develop red/green and temporarily disable the change to prove tests fail, then restore it exactly.
- Persist standing rules in place; keep task history and test-run results outside this file.
- Never publish releases, tags, packages or production deployments without explicit authorization.
