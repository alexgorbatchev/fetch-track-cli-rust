---
created_on: 2026-10-14 09:24
last_modified: 2026-10-14 22:40
status: current
---

# audio-metadata development

This internal Cargo workspace package belongs to `fetch-track-cli` and inherits `../../AGENTS.md` and the migration parent's Rust instructions. Run local recipes from this package directory. Consumers use its native public APIs directly; keep command parsing, terminal formatting and event schemas in consumers.

## Commands

- `just build`: build all targets.
- `just test`: run real local HTTP, native process and audio-file tests.
- `just fmt`: format this package only.
- `just check`: formatting, strict Clippy and at least 90% line coverage.
- FFmpeg and FFprobe must be on PATH for the behavioral suite. Keep generated audio, isolated caches and execution logs in `.tmp/`.

## Contracts

- Preserve native `OsStr` arguments and `Path` values through subprocess boundaries. Lossy conversion belongs only in display strings.
- `Tools` runs isolated Unix process groups with bounded output and cancellation. Kill descendants on cancellation, timeout and early output failure; do not leave blocking readers waiting on inherited pipes.
- Recognition derives AcoustID duration from actual decoded 16-bit mono samples at 11025 Hz. Do not require a previous probe or substitute a caller's estimated duration.
- Preserve the Shazam, AcoustID, iTunes and MusicBrainz resolution chain, artist/title identity checks, approved front artwork and full release dates. Propagate cancellation through every fallback. Unknown metadata remains unknown.
- Require a consumer-supplied HTTP user agent. MusicBrainz requests are serialized to at most one start per second per resolver. Keep local endpoints injectable for tests; never call live providers from tests.
- Cache recognition by file contents and query for seven days, AcoustID results by fingerprint and decoded duration for thirty days, and binary artwork by URL for thirty days. Check cancellation before cache hits. Use unique temporary files and atomic persistence.
- Merge container tags before first-audio-stream tags. Recognize actual attached pictures and native front covers, including Opus; do not classify ordinary video as cover artwork.
- Partial `TagUpdate` fields preserve unspecified metadata. Preserve encoded audio packets in every supported native container. Keep image decoding bounded and normalize front covers to square 1400px JPEG.
- Stage tag edits beside the destination; reject unrelated destination collisions and preserve source bytes on failure. Check cancellation before persistence. A successful write is the commit point; consumers decide whether to remove a source after a rename.
- Any runtime behavior change requires corresponding behavioral tests and at least 90% line coverage; `scripts/` is excluded. Develop red/green and disable the implementation temporarily to prove tests fail, then restore it exactly.
- Persist standing rules in place; keep task history and verification results outside this file.
- Never publish releases, tags, packages or production deployments without explicit authorization.
