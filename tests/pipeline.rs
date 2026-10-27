use audio_metadata::metadata::Endpoints;
use cli_event_stream::Reporter;
mod common;
use axum::{
    Json, Router,
    routing::{get, post},
};
use common::Fixture;
use fetch_track_cli::{CancellationToken, Error, pipeline::Options, progress::Event};
use serde_json::json;
use tokio::io::AsyncReadExt;

#[tokio::test]
async fn query_acquisition_emits_ordered_events_and_always_final_artifact() {
    let fixture = Fixture::new();
    let (writer, mut reader) = tokio::io::duplex(65536);
    let p = fixture.pipeline(Endpoints::default(), Reporter::writer(writer));
    let options = Options {
        out_dir: fixture.root().join("output"),
        sources: vec!["youtube".into(), "soundcloud".into()],
        skip_verify: true,
        skip_metadata: true,
        interactive: false,
        agent: true,
        verbose: false,
    };
    let token = CancellationToken::new();
    let result = p.acquire("Artist - Song", &options, &token).await.unwrap();
    assert!(result.path.ends_with("Artist - Song.flac"));
    assert!(fixture.root().join("output/Artist - Song.flac").is_file());
    assert!(!options.out_dir.join(".tmp").exists());
    let cached = p
        .search(&options.sources, "Artist Song", "Artist", "Song", &token)
        .await
        .unwrap();
    assert!(!cached.is_empty());
    drop(p);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    let events: Vec<Event> = bytes
        .split(|b| *b == b'\n')
        .filter(|b| !b.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    let selected = events
        .iter()
        .position(|e| e.kind == "candidate_selected")
        .unwrap();
    let found = events
        .iter()
        .position(|e| e.kind == "candidate_found")
        .unwrap();
    assert!(found < selected);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "candidate_selected")
            .count(),
        1
    );
    let complete = events.iter().find(|e| e.kind == "complete").unwrap();
    assert_eq!(complete.result.as_ref().unwrap().path, result.path);
    assert!(events.iter().any(|e| e.percent == Some(25.0)));
}

#[tokio::test]
async fn complete_pipeline_resolves_tags_and_verifies_with_local_services() {
    let fixture = Fixture::new();
    let app=Router::new().route("/shazam",post(||async{Json(json!({"track":{"title":"Song","subtitle":"Artist","images":{"coverart":""}}}))})).route("/itunes",get(||async{Json(json!({"results":[{"trackName":"Song","artistName":"Artist","collectionName":"Album","releaseDate":"1999-01-02T00:00:00Z"}]}))})).route("/mb",get(||async{Json(json!({"recordings":[]}))}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let url = |p: &str| format!("{base}{p}").parse().unwrap();
    let endpoints = Endpoints {
        itunes: url("itunes"),
        musicbrainz: url("mb"),
        shazam: shazam_client::Endpoints {
            recognition: Some(url("shazam")),
            ..Default::default()
        },
        ..Default::default()
    };
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let p = fixture.pipeline(endpoints, Reporter::default());
    let options = Options {
        out_dir: fixture.root().join("output"),
        sources: vec!["youtube".into()],
        skip_verify: false,
        skip_metadata: false,
        interactive: false,
        agent: true,
        verbose: false,
    };
    let result = p
        .acquire(
            "https://example.org/audio",
            &options,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.title, "Song");
    assert_eq!(result.artist, "Artist");
    assert_eq!(result.album, "Album");
    assert_eq!(result.release_year, "1999");
    assert!(result.duration_seconds > 7.0);
    assert!(result.bandwidth_hz > 13000);
    server.abort();
}

#[test]
fn cli_runs_root_workflow_ndjson_stdout_dependency_checks_and_verify() {
    let fixture = Fixture::new();
    let out = fixture
        .cli()
        .args([
            "Artist - Song",
            "--out-dir",
            "output",
            "--skip-verify",
            "--skip-metadata",
            "--progress-target",
            "stdout",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let events: Vec<Event> = out
        .stdout
        .split(|b| *b == b'\n')
        .filter(|b| !b.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    assert_eq!(events.last().unwrap().kind, "complete");
    let out = fixture
        .cli()
        .args(["dependencies", "check"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ffprobe: OK"));
    let out = fixture
        .cli()
        .args(["track", "verify"])
        .arg(&fixture.audio)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["metadata"]["codec"], "flac");
    assert_eq!(String::from_utf8_lossy(&out.stderr).lines().count(), 1);
    let out = fixture
        .cli()
        .args([
            "Artist - Song",
            "--out-dir",
            "output",
            "--skip-verify",
            "--skip-metadata",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("exists"));
    std::fs::remove_file(fixture.bin.join("yt-dlp")).unwrap();
    let out = fixture.cli().arg("Artist - Song").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("2024.08.01"));
}

#[tokio::test]
async fn search_missing_sources_cancel_and_no_candidate_are_typed() {
    let fixture = Fixture::new();
    let p = fixture.pipeline(Endpoints::default(), Reporter::default());
    let token = CancellationToken::new();
    assert!(
        p.search(&["invalid".into()], "query", "", "query", &token)
            .await
            .is_err()
    );
    assert!(matches!(
        p.search(&["youtube".into()], "unrelated", "", "unrelated", &token)
            .await,
        Err(Error::NoCandidate)
    ));
    assert!(matches!(
        p.search(&[], "query", "", "query", &token).await,
        Err(Error::NoCandidate)
    ));
    token.cancel();
    assert!(matches!(
        p.search(&["youtube".into()], "query", "", "query", &token)
            .await,
        Err(Error::Cancelled)
    ));
}

#[tokio::test]
async fn download_reports_exact_path_retry_boundaries_and_rejects_escaped_files() {
    let fixture = Fixture::new();
    let p = fixture.pipeline(Endpoints::default(), Reporter::default());
    let stage = fixture.root().join("stage");
    let token = CancellationToken::new();
    let script = fixture.bin.join("yt-dlp");
    let original = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    assert!(
        p.download("source", &stage, &token)
            .await
            .unwrap_err()
            .to_string()
            .contains("did not report")
    );
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' '__FETCH_PATH__\"absent.flac\"'\n",
    )
    .unwrap();
    assert!(
        p.download("source", &stage, &token)
            .await
            .unwrap_err()
            .to_string()
            .contains("missing output")
    );
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' '__FETCH_PATH__{}'\n",
            serde_json::to_string(&fixture.audio.to_string_lossy()).unwrap()
        ),
    )
    .unwrap();
    assert!(
        p.download("source", &stage, &token)
            .await
            .unwrap_err()
            .to_string()
            .contains("escaped")
    );
    let attempts = fixture.root().join("attempts");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf 'attempt\\n' >> '{}'\nexit 1\n",
            attempts.display()
        ),
    )
    .unwrap();
    assert!(p.download("source", &stage, &token).await.is_err());
    assert_eq!(
        std::fs::read_to_string(attempts).unwrap().lines().count(),
        2
    );
    std::fs::write(&script, original).unwrap();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        p.download("source", &stage, &cancelled).await,
        Err(Error::Cancelled)
    ));
}

#[tokio::test]
async fn url_probe_failure_falls_back_and_interactive_requires_actual_terminal() {
    let fixture = Fixture::new();
    let p = fixture.pipeline(Endpoints::default(), Reporter::default());
    let script = fixture.bin.join("yt-dlp");
    let original = std::fs::read_to_string(&script).unwrap();
    std::fs::write(
        &script,
        original.replace(
            "set -eu\n",
            "set -eu\nif [ \"$1\" = '--dump-single-json' ]; then exit 1; fi\n",
        ),
    )
    .unwrap();
    let mut options = Options {
        out_dir: fixture.root().join("output"),
        sources: vec![],
        skip_verify: true,
        skip_metadata: true,
        interactive: false,
        agent: false,
        verbose: true,
    };
    let result = p
        .acquire(
            "https://example.org/audio",
            &options,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.path,
        fetch_track_cli::display(&options.out_dir.join("Artist - Song.flac"))
    );
    assert!(options.out_dir.join("Artist - Song.flac").exists());
    options.sources = vec!["youtube".into()];
    options.interactive = true;
    assert!(
        p.acquire("Artist - Song", &options, &CancellationToken::new())
            .await
            .unwrap_err()
            .to_string()
            .contains("requires a terminal")
    );
    assert!(
        p.probe_url("https://example.org/audio", &CancellationToken::new())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn verification_failure_keeps_master_and_still_completes() {
    let fixture = Fixture::new();
    let p = fixture.pipeline(Endpoints::default(), Reporter::default());
    std::fs::remove_file(fixture.bin.join("ffmpeg")).unwrap();
    std::os::unix::fs::symlink("/usr/bin/false", fixture.bin.join("ffmpeg")).unwrap();
    let options = Options {
        out_dir: fixture.root().join("output"),
        sources: vec!["youtube".into()],
        skip_verify: false,
        skip_metadata: true,
        interactive: false,
        agent: true,
        verbose: true,
    };
    let result = p
        .acquire("Artist - Song", &options, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        result.path,
        fetch_track_cli::display(&options.out_dir.join("Artist - Song.flac"))
    );
    assert!(options.out_dir.join("Artist - Song.flac").is_file());
    assert_eq!(result.bandwidth_hz, 0);
}

#[test]
fn cli_validates_progress_environment_before_dependencies_and_handles_sigterm() {
    use std::time::{Duration, Instant};
    let fixture = Fixture::new();
    let out = fixture
        .cli()
        .arg("query")
        .env("FETCH_TRACK_PROGRESS_TARGET", "fd://-1")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid progress target"));
    let script = fixture.bin.join("yt-dlp");
    let ready = fixture.root().join("ready");
    std::fs::write(script,format!("#!/bin/sh\nif [ \"$1\" = '--version' ]; then printf '2026.10.01\\n'; exit 0; fi\nprintf 'ready' > '{}'\n/bin/sleep 30\n",ready.display())).unwrap();
    let child = fixture
        .cli()
        .args(["query", "--skip-verify", "--skip-metadata"])
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ready.exists());
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(130));
}
