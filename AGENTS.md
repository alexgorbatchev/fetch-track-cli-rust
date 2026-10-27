---
created_on: 2026-10-14 08:25
last_modified: 2026-10-14 22:40
status: current
---

# fetch-track Rust development

Inherit the parent Rust instructions. This Git repository contains a Cargo workspace with the CLI, `crates/audio-metadata`, and `crates/cli-event-stream`; shared dependencies outside this repository remain native path dependencies.

## Development

- Build the distributable executable: `just build` (outputs `bin/fetch-track`).
- Run locally: `just run <args>` or `just run-ai <args>`.
- Format only this package: `just fmt`.
- Run behavioral tests: `just test`.
- Run formatting, strict Clippy, and at least 90% line coverage: `just check`.
- Keep evidence under `.tmp/`; use real local HTTP servers, FFmpeg-generated audio, and isolated subprocess fixtures.
- Read CLI, documentation, and skill-writing guidance before changing their corresponding interfaces.

## Architecture

- `src/cli.rs`: native Clap tree, dependency commands, dispatch and typed exit classification.
- `src/pipeline.rs` and `src/pipeline/`: candidate workflow, yt-dlp processes, probing, download and verification.
- The internal `crates/audio-metadata` package supplies provider resolution, native inspection, content-aware caching, image processing and atomic tag edits. Use its APIs directly rather than introducing re-export facades.
- `src/quality.rs`: RustFFT spectrum, calibrated signal measurements and structural heuristics.
- `audio_metadata::process::Tools` supplies bounded process output, deadlines and process-group cancellation with native `OsStr` arguments.
- `src/progress.rs` owns exact domain event schemas; the internal `crates/cli-event-stream` package owns native descriptor and socket transport.
- `src/update.rs`: explicit unpublished-by-default Rust release channel and atomic executable replacement.

## Contracts

- Keep root query acquisition supported. Group maintenance and verification under their subjects.
- Preserve raw version output and exact offline `skill` pointer bytes. Keep the canonical source pointer distinct from publication status.
- Update `skills/fetch-track/` in the same change as command, flag, environment, output or side-effect changes. The documentation test compares it against the live Clap tree.
- Print returned runtime errors exactly once from `main`; use typed exit classification. Fold home paths in displayed artifacts and diagnostics.
- Preserve encoded audio packets. Select native containers from the actual codec and use FFmpeg stream copy when remuxing; never hide a transcoding step.
- Use the output directory exactly as specified, including `.`. Preserve downloaded masters if optional metadata or verification fails, and never overwrite an unrelated output.
- Retain complete progress results even when verification and metadata are skipped; emit candidate selection only after selection.
- Keep cache keys content-aware for recognition, check cancellation before cache hits, and avoid storing credentials.
- Verify spectral changes using synthetic signals at their actual sample rate; do not infer bandwidth from aliased subsamples.
- Treat mix structure as heuristics. Silence and insufficient samples cannot establish bandwidth.
- Signal cancellation must kill the entire child process group, including extractor descendants.
- Keep Rust artifacts unpublished until an authorized release exists. Never install Go release archives as Rust binaries or add an invented default release URL.
- Self-update tests may replace only a copied fixture executable. Never run the original Go self-update tests against the development executable.
- Dependency installation tests must use configured local release fixtures; never invoke system package managers from tests.
