`audio-metadata` recognizes audio recordings, retrieves release metadata and edits native audio tags for Rust tools. It provides shared file and provider behavior for applications that need to preserve their encoded audio.

# What It Does

- **Recognition**: Resolves recordings through Shazam, AcoustID, iTunes and MusicBrainz, with filename metadata as the final fallback.
- **Inspection**: Reads audio properties, container and stream tags, and front-cover dimensions.
- **Atomic edits**: Writes complete metadata or partial tag updates while preserving encoded audio packets.
- **Artwork**: Downloads bounded cover images and embeds a square 1400px JPEG front cover.
- **Caching**: Reuses content-aware recognition results and downloaded artwork.

# How It Works

1. Supply executable search paths, a cache location and your application's HTTP user agent.
2. Inspect a local file or resolve its recording metadata with a cancellation token.
3. Apply selected tag fields to an explicit destination, or create a cleaned metadata-derived filename.

# How it Really Works

- Recognition sends acoustic fingerprints to Shazam and AcoustID, then sends artist/title search queries to iTunes and MusicBrainz when enrichment is needed. AcoustID duration comes from decoded samples. Results with unknown album, genre or release date leave those fields empty.
- Network operations have fifteen-second request deadlines. MusicBrainz request starts are spaced at least one second apart per resolver. JSON responses are limited to 8 MiB and artwork to 20 MiB. All provider endpoints are configurable.
- Enabled caches store recognition for seven days under the supplied directory, keyed by file contents and query. AcoustID results are also reused for thirty days by acoustic fingerprint and decoded duration, including across lossless encodings of the same recording. Artwork is cached for thirty days by URL. Disabled caches perform no disk operations. Corrupt and expired entries are misses.
- Inspection merges container tags with first-audio-stream tags, preserving container precedence. Ordinary video streams are not cover art. Native front covers in Opus and other audio formats are recognized.
- Tag editing supports MP3, M4A, Opus, FLAC and Ogg. An absent `TagUpdate` field retains its existing value; a present empty string clears that field. Front covers are replaced while other pictures remain intact. Images are limited to 16000px per dimension and 256 MiB of decoding allocation.
- A destination is replaced only when it is the source itself; unrelated collisions fail. Edits stage a copy beside the destination and commit atomically. Failures preserve the source; successful writes leave source removal to the caller. Synchronous tag and image work blocks its calling thread.
- Native file paths remain intact through subprocess arguments. Display names may replace invalid UTF-8 characters. The library reads no environment variables automatically and performs no work at import time.

# Prerequisites

- Rust 1.88 or newer on Unix, with a Tokio runtime for asynchronous operations.
- [FFmpeg and FFprobe](https://ffmpeg.org/download.html) available through the supplied executable search path.
- Network access to the selected metadata providers for recognition and artwork downloads.

# Installation

This unpublished implementation crate belongs to the `fetch-track-cli` repository. From a consumer beside that repository:

```sh
cargo add audio-metadata --path ../fetch-track-cli/crates/audio-metadata
```

# Quick Start

Inspect a local audio file without contacting metadata providers:

```rust
use audio_metadata::{CancellationToken, inspect, process::Tools};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tools = Tools::new(std::env::var_os("PATH").unwrap_or_default(), std::env::current_dir()?);
    let file = std::path::Path::new("track.flac");
    let info = inspect::inspect(&tools, file, &CancellationToken::new()).await?;
    println!("{} - {} ({:.1}s)", info.artist, info.title, info.duration_seconds);
    Ok(())
}
```

# API

| Entry point | Use |
| :--- | :--- |
| `inspect::inspect(&Tools, &Path, &CancellationToken)` | Returns native audio properties, common fields, all merged tags and artwork dimensions. |
| `metadata::Resolver::new(tools, cache, endpoints, user_agent)` | Creates an HTTP provider client with explicit configuration. |
| `Resolver::resolve(path, query, artist, title, token)` | Resolves recording identity and enriches metadata. |
| `Resolver::{shazam,acoustid,itunes,musicbrainz}` | Calls a selected recognition or search provider directly. |
| `Resolver::artwork(url, token)` | Returns bounded downloaded or cached bytes. |
| `tag::write(source, target, &TagUpdate, token)` | Applies a partial update and returns the committed destination. |
| `tag::apply(source, &Metadata, out_dir, artwork, token)` | Creates a metadata-derived filename and provenance comment. |
| `tag::clean_output(source, out_dir)` | Copies bytes into a cleaned filename without modifying tags. |
| `process::Tools::{run,lines}` | Executes bounded, cancellable processes with native arguments. |
| `cache::Cache` | Provides atomic JSON and bounded binary entries with explicit lifetimes. |
| `identity::normalize` | Normalizes Unicode identity strings for matching. |
| `Error` | Distinguishes cancellation, invalid inputs, process, metadata, I/O, JSON and HTTP failures. |

# License

[MIT License](LICENSE) (c) 2026 Alex Gorbatchev.
