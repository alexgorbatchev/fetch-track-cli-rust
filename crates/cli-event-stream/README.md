`cli-event-stream` delivers JSON progress records from Rust command-line tools to sockets, files and inherited descriptors. It lets cooperating processes share a cancellable event stream while retaining their own event schemas.

# What It Does

- **Native destinations**: Connects to Unix sockets, TCP addresses, stdout, stderr and inherited descriptors.
- **Complete records**: Serializes concurrent callers into newline-delimited JSON records.
- **Cancellation**: Cancels blocked socket and pipe writes without holding the runtime open.
- **Descriptor ownership**: Leaves caller descriptors open and restores their original flags when the final reporter drops.

# How It Works

1. Parse a destination and connect a reporter inside a Tokio runtime.
2. Pass any serializable record to `emit` with a cancellation token.
3. Share reporter clones with tasks and drop them when delivery finishes.

# How it Really Works

- Clones share a writer and serialize each record with its newline. Successful emission includes flushing the writer; it does not establish that the receiving application processed the record.
- Connections have a ten-second deadline. Write operations have cancellation but no fixed deadline; the caller controls when to cancel.
- Cancellation or a write failure can leave a partial final record. Stop using that stream after a failed emission; consumers should discard an incomplete trailing JSON line.
- A selected inherited descriptor must not be concurrently written or reconfigured. Pipes and sockets temporarily share nonblocking flags with the original descriptor; the last reporter restores its original flags. Dropping a duplicate does not shut down the caller's socket.
- No environment variables are read, no event history is retained and no connections occur at import time. The default reporter discards events without serializing them.

# Prerequisites

- Rust 1.88 or newer and a Unix platform.
- A Tokio runtime with I/O and time enabled.

# Installation

This unpublished implementation crate belongs to the `fetch-track-cli` repository. From a consumer beside that repository:

```sh
cargo add cli-event-stream --path ../fetch-track-cli/crates/cli-event-stream
```

# Quick Start

```rust
use cli_event_stream::{CancellationToken, Reporter, Target};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = CancellationToken::new();
    let reporter = Reporter::connect(&Target::Stdout, &token).await?;
    reporter.emit(serde_json::json!({"type": "complete", "path": "track.flac"}), &token).await?;
    Ok(())
}
```

# API

| Export | Use |
| :--- | :--- |
| `Target::parse(&str)` | Accepts `stdout`, `stderr`, `fd://N`, `unix://PATH`, `tcp://HOST:PORT`, socket paths and host/port shorthand. |
| `Reporter::connect(&Target, &CancellationToken)` | Creates a shared asynchronous writer. |
| `Reporter::writer(writer)` | Uses a caller-provided asynchronous writer. |
| `Reporter::emit(record, &CancellationToken)` | Serializes and flushes one record, returning a typed error on failure. |
| `Reporter::default()` | Creates an inactive reporter. |
| `Error` | Distinguishes invalid targets, cancellation, I/O and serialization failures. |

# License

[MIT License](LICENSE) (c) 2026 Alex Gorbatchev.
