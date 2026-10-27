# Commands and flags

| Command | Arguments and behavior |
| :--- | :--- |
| `fetch-track` | `[QUERY]...`, joined by spaces; artist/title text or URL. No query shows help. |
| `fetch-track track` | Verification subject; a verb is required. |
| `fetch-track track verify` | Required `<TARGET>...`, joined by spaces; local file or URL. |
| `fetch-track dependencies` | Check versions, equivalent to `dependencies check`. |
| `fetch-track dependencies check` | Report installed and minimum versions; fail if unsatisfied. |
| `fetch-track dependencies install` | Optional `[DEPENDENCY]...`; named tools or all unsatisfied tools. |
| `fetch-track dependencies update` | Optional `[DEPENDENCY]...`; named tools or all tools. |
| `fetch-track self` | Executable maintenance subject; a verb is required. |
| `fetch-track self update` | Replace executable from configured published Rust channel. |
| `fetch-track skill` | Print exact canonical installation pointer offline. |
| `fetch-track help` | Optional `[COMMAND]...`; show command help. |

DEPENDENCY values are `yt-dlp`, `ffmpeg`, and `ffprobe`.

Clap also generates these help paths; each displays help for the named command without running it:

- `fetch-track help track`; `fetch-track help track verify`.
- `fetch-track help dependencies`; `fetch-track help dependencies check`; `fetch-track help dependencies install`; `fetch-track help dependencies update`.
- `fetch-track help self`; `fetch-track help self update`; `fetch-track help skill`; `fetch-track help help`.
- `fetch-track track help`; `fetch-track track help verify`; `fetch-track track help help`.
- `fetch-track dependencies help`; `fetch-track dependencies help check`; `fetch-track dependencies help install`; `fetch-track dependencies help update`; `fetch-track dependencies help help`.
- `fetch-track self help`; `fetch-track self help update`; `fetch-track self help help`.

| Acquisition flag | Type, default and behavior |
| :--- | :--- |
| `--out-dir`, `-o` | string, default `.`; DIRECTORY receives output. |
| `--sources`, `-s` | string, default `youtube,soundcloud`; SOURCES is comma-separated `youtube` and/or `soundcloud`. |
| `--skip-verify` | bool, default false; skip audio analysis. |
| `--skip-metadata` | bool, default false; skip lookup, artwork and tagging. |
| `--interactive`, `-i` | bool, default false; choose from candidates in a human terminal. |
| `--no-cache` | bool, default false; bypass cache reads and writes. |
| `--progress-target` | string, unset; URI selects NDJSON destination. |
| `--progress-socket` | string, unset; PATH selects a Unix socket. |

| Global flag | Type, default and behavior |
| :--- | :--- |
| `--auto-install` | bool, default false; install unsatisfied tools without application confirmation. |
| `--verbose`, `-v` | bool, default false; print detailed diagnostics. |
| `--version` | bool, default false; print raw package version and newline. |
| `--help`, `-h` | bool, default false; print the selected command's help. |

Direct HTTP(S) URLs can use any extractor supported by yt-dlp, including Bandcamp. Bare known source domains are recognized as URLs. Search is restricted to YouTube and SoundCloud. An interactive choice requires human mode and a terminal. Explicit existing outputs fail rather than overwrite files.

`AGENT=1` enables plain agent output and a skill alert at the start of help. Human mode uses an ASCII spinner on stderr when verbose mode is disabled. Global flags may appear on subject commands. Root acquisition flags belong before any subject command.
