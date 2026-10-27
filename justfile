mod audio_metadata 'crates/audio-metadata/justfile'
mod cli_event_stream 'crates/cli-event-stream/justfile'

default:
    @just --list

[private]
_fmt:
    cargo fmt --package fetch-track-cli

[private]
_test:
    mkdir -p .tmp
    cargo test --package fetch-track-cli --all-targets

[private]
_fmt-check:
    cargo fmt --package fetch-track-cli -- --check

[private]
_lint:
    cargo clippy --package fetch-track-cli --all-targets -- -D warnings

coverage:
    mkdir -p .tmp
    cargo llvm-cov --package fetch-track-cli --exclude-from-report audio-metadata --exclude-from-report cli-event-stream --all-targets --fail-under-lines 90

check: _fmt-check _lint _test coverage
    just audio_metadata check
    just cli_event_stream check

build:
    cargo build --package fetch-track-cli --release --bin fetch-track
    mkdir -p bin
    cp target/release/fetch-track bin/fetch-track

run *args:
    cargo run --package fetch-track-cli --bin fetch-track -- {{ args }}

run-ai *args:
    AGENT=1 cargo run --package fetch-track-cli --bin fetch-track -- {{ args }}

test: _test
    just audio_metadata test
    just cli_event_stream test

lint: _lint
    just audio_metadata lint
    just cli_event_stream lint

fmt: _fmt
    just audio_metadata fmt
    just cli_event_stream fmt

fmt-check: _fmt-check
    just audio_metadata fmt-check
    just cli_event_stream fmt-check
