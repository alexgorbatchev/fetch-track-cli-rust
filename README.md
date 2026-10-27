Find audio tracks, download their original encoded stream, inspect their quality, and attach recognized metadata and artwork.

# What It Does

- Searches YouTube and SoundCloud, or accepts a direct URL supported by yt-dlp.
- Ranks candidates using artist, title, duration, and mix descriptions.
- Measures bandwidth, peak, RMS, clipping, and suggested gain; identifies likely radio edits from title and duration.
- Recognizes recordings through Shazam and AcoustID, enriches them through iTunes and MusicBrainz, and writes tags with square cover artwork.
- Streams structured progress for tools such as fetch-mix.

# How It Works

Search sources run concurrently. The highest ranked candidate is selected, or a terminal prompt lets you choose with `--interactive`. Downloads retain the source codec; an unsuitable container is remuxed with FFmpeg stream copy. Optional verification and metadata then run against the downloaded audio.

# How it Really Works

The default output directory is the current directory. Existing files are protected. A clean downloaded master is saved before optional verification or metadata, so a later warning does not discard it. Tag updates are staged and replaced atomically; a metadata-derived filename can leave both the downloaded master and tagged copy.

Quality inspection analyzes up to 30 seconds of decoded audio at its real sample rate. Spectral bandwidth is an estimate, not proof of provenance. Radio-edit, extended-mix, intro and outro classifications use title and duration heuristics rather than beat detection. Silence reports unknown quality.

Recognition sends audio-derived signatures or fingerprints to external providers. Search and metadata services receive textual queries. Artwork is fetched from provider URLs. Search results are cached for 12 hours, URL metadata for 24 hours, and recording metadata for seven days under the platform cache directory's `fetch-track` folder. `--no-cache` bypasses reads and writes.

# Installation

This Rust migration is currently local and unpublished. No Rust release downloads are available. Existing Go release archives are not Rust binaries. Contributor build instructions are in [AGENTS.md](AGENTS.md).

The executable currently targets Unix platforms. Runtime tools are yt-dlp 2024.08.01 or newer and FFmpeg/ffprobe 4.4 or newer.

# Setup

Check tools with `fetch-track dependencies check`. Install missing tools with `fetch-track dependencies install`, or allow installation during a command with `--auto-install`. yt-dlp is installed into the application's managed binary directory. FFmpeg installation uses the platform package manager and can request system privileges. Human terminal sessions offer a confirmation prompt for missing tools; agent and redirected sessions require explicit installation or `--auto-install`.

`fetch-track skill` prints an offline canonical skill-installation pointer. The matching local guide is [skills/fetch-track/SKILL.md](skills/fetch-track/SKILL.md); publication of the Rust guide is pending.

# Quick Start

```sh
fetch-track 'Artist - Track Title' --out-dir ./music
fetch-track 'https://soundcloud.com/artist/track' --skip-metadata
fetch-track track verify './music/Artist - Track Title.flac'
AGENT=1 fetch-track 'Artist - Track Title' --progress-target stdout
fetch-track dependencies check
```

Successful human acquisition ends with `DONE: <path>`. Agent acquisition normally prints `output: <path>` and `status: <status>`. When progress targets stdout, acquisition stdout contains only NDJSON. Human progress and warnings go to stderr. Agent verification prints a JSON report; human verification prints measurements. Displayed home paths use `~/`.

# Commands

| Command | Behavior |
| :--- | :--- |
| `fetch-track <QUERY>...` | Search, download, optionally verify and tag; quoted URL or query recommended. |
| `fetch-track track verify <TARGET>...` | Inspect a local file or download and inspect a URL. |
| `fetch-track dependencies` / `dependencies check` | Report installed and required versions. |
| `fetch-track dependencies install [DEPENDENCY]...` | Install named tools, or missing/outdated tools. |
| `fetch-track dependencies update [DEPENDENCY]...` | Update named tools, or all tools. |
| `fetch-track self update` | Update from an explicitly configured Rust release channel. |
| `fetch-track skill` | Print the canonical skill pointer without accessing the network. |
| `fetch-track help [COMMAND]...` | Show command help. |

# Options & Flags

| Flag | Short | Default | Description |
| :--- | :--- | :--- | :--- |
| `--out-dir <DIRECTORY>` | `-o` | `.` | Destination directory. |
| `--sources <SOURCES>` | `-s` | `youtube,soundcloud` | Comma-separated search sources. Bandcamp accepts direct URLs. |
| `--skip-verify` | | `false` | Skip quality and structure inspection. |
| `--skip-metadata` | | `false` | Skip recognition, artwork and tags. |
| `--interactive` | `-i` | `false` | Choose candidates in a human terminal. |
| `--no-cache` | | `false` | Bypass cache reads and writes. |
| `--progress-target <URI>` | | unset | NDJSON destination: Unix socket, TCP, inherited descriptor, stdout or stderr. |
| `--progress-socket <PATH>` | | unset | Unix socket shortcut. |
| `--auto-install` | | `false` | Install missing/outdated tools without the application prompt. |
| `--verbose` | `-v` | `false` | Print detailed diagnostics instead of a spinner. |
| `--version` | | `false` | Print raw version and exit. |
| `--help` | `-h` | `false` | Show help. |

The first nine flags apply to root acquisition. `--auto-install`, `--verbose`, and `--version` are global. Every command supports help.

# Automation and Updates

Set `AGENT=1` for plain output and skill alerts. `FETCH_TRACK_PROGRESS_TARGET` supplies a progress destination; `--progress-target` takes precedence, followed by `--progress-socket`. See the [progress contract](skills/fetch-track/references/progress.md).

Reserve an inherited progress descriptor for this command rather than writing to or reconfiguring it concurrently. Pipes and sockets become nonblocking during reporting; original flags are restored on final cleanup. Cancellation under backpressure can leave an incomplete final NDJSON line, which consumers discard.

Self-update returns an unpublished-release error by default. A deployed Rust channel requires all three environment variables: `FETCH_TRACK_RUST_RELEASE_BASE_URL`, `FETCH_TRACK_RUST_RELEASE_OWNER`, and `FETCH_TRACK_RUST_RELEASE_REPO`. The base must use HTTPS, except HTTP on loopback. The channel must provide the release redirect and platform archives documented in [setup](skills/fetch-track/references/setup.md). It must contain Rust executables, not Go artifacts.

Exit statuses are 0 for success, 1 for runtime failures, 2 for argument validation or a radio-edit verification result, and 130 for cancellation. Acquisition treats optional verification and metadata failures as warnings. Ctrl-C and SIGTERM cancel work and terminate child process groups.

# Migration Corrections

The Rust implementation corrects aliased spectral analysis, avoids forced lossy audio conversion, rejects unsupported Bandcamp search syntax, preserves case-sensitive source IDs, prevents misleading metadata artist matches, and retains a complete output result when optional phases are skipped. Provider integration is exercised against local HTTP servers; live provider availability and recognition accuracy are not guaranteed by those tests.

# License

MIT License (c) 2026 Alex Gorbatchev. See [LICENSE](LICENSE).
