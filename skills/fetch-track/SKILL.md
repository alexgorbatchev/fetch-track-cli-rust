---
name: fetch-track
description: >-
  Find, download, verify, and tag audio using the Rust fetch-track CLI. Use when
  fetching a track by artist/title or URL, inspecting audio quality, managing
  its external tools, or consuming its structured progress in batch workflows.
author: Alex Gorbatchev
metadata:
  created_on: 2026-10-14 08:25
  last_modified: 2026-10-14 08:35
  status: current
---

# fetch-track

Use `AGENT=1` for agent commands. Read [commands](references/commands.md) for the complete command and flag interface, [progress](references/progress.md) for NDJSON transports and events, and [setup](references/setup.md) before installing tools or updating the executable.

This is the local Rust guide. Rust artifacts and this revised guide are unpublished. `AGENT=1 fetch-track skill` prints the canonical source installation pointer offline; that pointer is distinct from publication of the Rust migration.

## Workflow

1. Check tools with `AGENT=1 fetch-track dependencies check`.
2. Acquire with `AGENT=1 fetch-track 'Artist - Title' --out-dir ./music` or a quoted direct URL.
3. Inspect a local file with `AGENT=1 fetch-track track verify ./music/file.flac`.
4. For batch orchestration, pass `--progress-target` and consume the complete event's `result.path`.

Default output is `.`. Existing files are protected; downloaded masters remain available if optional enrichment fails. Audio stays in its original codec and may be remuxed into a suitable native container. Metadata can create a tagged copy under a new artist/title filename while preserving the master.

Recognition contacts Shazam and AcoustID with audio-derived signatures or fingerprints, and iTunes and MusicBrainz with queries. Cache location is the platform cache directory plus `fetch-track`. `--no-cache` bypasses reading and writing. Search TTL is 12 hours, URL metadata 24 hours, recording metadata seven days.

Verification measures spectrum and signal levels from up to 30 seconds. Structure classifications use title and duration heuristics. Treat measured bandwidth as an estimate; silence yields unknown quality.

## Results and exits

Acquisition normally prints `output: <path>` and `status: <status>` in agent mode. A stdout progress target produces only NDJSON on acquisition stdout. Verification prints one JSON report in agent mode. Warnings, verbose diagnostics and runtime errors use stderr. Home paths are displayed with `~/`; expand them before opening files from another program.

Exit 0 means success, 1 a runtime failure, 2 argument validation or a likely radio edit from `track verify`, and 130 cancellation. Optional verification or metadata failures during acquisition are warnings; the complete event still contains the retained output path. Ctrl-C and SIGTERM cancel work and terminate child process groups.
