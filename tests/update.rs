use axum::{Router, response::Redirect, routing::get};
use fetch_track_cli::update::from_values;

fn archive() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    let bytes = b"#!/bin/sh\nprintf 'new-version\\n'\n";
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, "fetch-track", &bytes[..])
        .unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}

#[test]
fn release_configuration_validates_complete_safe_channels() {
    assert!(from_values(None, None, None).unwrap().is_none());
    for (base, owner, repo) in [
        (Some("https://example.org"), None, None),
        (Some("http://example.org"), Some("owner"), Some("repo")),
        (
            Some("https://user@example.org"),
            Some("owner"),
            Some("repo"),
        ),
        (
            Some("https://example.org?token=x"),
            Some("owner"),
            Some("repo"),
        ),
        (Some("::::"), Some("owner"), Some("repo")),
        (Some("https://example.org"), Some(".."), Some("repo")),
        (Some("https://example.org"), Some("owner"), Some("a/b")),
    ] {
        assert!(
            from_values(
                base.map(str::to_owned),
                owner.map(str::to_owned),
                repo.map(str::to_owned)
            )
            .is_err()
        );
    }
    assert!(
        from_values(
            Some("https://example.org".into()),
            Some("owner".into()),
            Some("rust-track".into())
        )
        .unwrap()
        .is_some()
    );
    assert!(
        from_values(
            Some("http://localhost:80".into()),
            Some("owner".into()),
            Some("rust-track".into())
        )
        .unwrap()
        .is_some()
    );
}

#[tokio::test]
async fn native_self_replace_updates_a_copied_cli_only() {
    let bytes = archive();
    let app = Router::new()
        .route(
            "/owner/rust-track/releases/latest",
            get(|| async { Redirect::to("/owner/rust-track/releases/tag/v9.0.0") }),
        )
        .route(
            "/owner/rust-track/releases/download/v9.0.0/{asset}",
            get(move || {
                let bytes = bytes.clone();
                async move { bytes }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let executable = temp.path().join("fetch-track");
    std::fs::copy(env!("CARGO_BIN_EXE_fetch-track"), &executable).unwrap();
    let source_before = std::fs::metadata(env!("CARGO_BIN_EXE_fetch-track"))
        .unwrap()
        .len();
    let output = tokio::process::Command::new(&executable)
        .args(["self", "update"])
        .env("FETCH_TRACK_RUST_RELEASE_BASE_URL", base)
        .env("FETCH_TRACK_RUST_RELEASE_OWNER", "owner")
        .env("FETCH_TRACK_RUST_RELEASE_REPO", "rust-track")
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"OK: updated to 9.0.0\n");
    let updated = std::process::Command::new(&executable)
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(updated.stdout, b"new-version\n");
    assert_eq!(
        std::fs::metadata(env!("CARGO_BIN_EXE_fetch-track"))
            .unwrap()
            .len(),
        source_before
    );
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    server.abort();
}
