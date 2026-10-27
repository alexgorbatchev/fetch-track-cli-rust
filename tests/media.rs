use audio_metadata::{
    cache::Cache,
    metadata::{Endpoints, Resolver},
    process::Tools,
};
use cli_event_stream::Reporter;
use fetch_track_cli::{
    CancellationToken,
    pipeline::{Pipeline, Temporary},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn media(directory: &Path, ext: &str, codec: &str) -> PathBuf {
    let output = directory.join(format!("Artist - Title [video-id].{ext}"));
    let result = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=14000:sample_rate=48000:duration=2",
            "-c:a",
            codec,
            "-y",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    output
}
fn hash(path: &Path) -> Vec<u8> {
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:a:0", "-c:a", "copy", "-f", "hash", "-hash", "sha256", "-",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
fn pipeline(directory: &Path) -> Pipeline {
    let tools = Tools::new(std::env::var_os("PATH").unwrap(), directory.to_owned());
    let cache = Cache::new(directory.join("cache"), false);
    let resolver = Resolver::new(
        tools.clone(),
        cache.clone(),
        Endpoints::default(),
        "fetch-track/0.1.0 (https://github.com/alexgorbatchev/fetch-track-cli)",
    )
    .unwrap();
    Pipeline {
        tools,
        cache,
        resolver,
        reporter: Reporter::default(),
    }
}
#[tokio::test]
async fn real_ffmpeg_probe_pcm_and_temporary_cleanup() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let p = pipeline(temp.path());
    let file = media(temp.path(), "flac", "flac");
    let report = p
        .verify(file.to_str().unwrap(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.metadata.codec, "flac");
    assert!((report.metadata.duration - 2.0).abs() < 0.01);
    assert!(report.quality.bandwidth_hz > 13000);
    assert!(report.structure.radio_edit);
    assert_eq!(report.status, "WARNING");
    use std::os::unix::ffi::OsStringExt;
    let native = temp
        .path()
        .join(std::ffi::OsString::from_vec(b"native-\xff.flac".to_vec()));
    std::fs::copy(&file, &native).unwrap();
    let native_report = p
        .verify_file(&native, &CancellationToken::new())
        .await
        .unwrap();
    assert!(native_report.quality.bandwidth_hz > 13000);
    assert!(
        p.verify_file(&temp.path().join("absent"), &CancellationToken::new())
            .await
            .is_err()
    );
    {
        let work = Temporary::new(temp.path()).unwrap();
        std::fs::write(work.path().join("owned"), b"own").unwrap();
        assert!(temp.path().join(".tmp").exists());
    }
    assert!(!temp.path().join(".tmp").exists());
    std::fs::create_dir(temp.path().join(".tmp")).unwrap();
    std::fs::write(temp.path().join(".tmp/unrelated"), b"keep").unwrap();
    drop(Temporary::new(temp.path()).unwrap());
    assert_eq!(
        std::fs::read(temp.path().join(".tmp/unrelated")).unwrap(),
        b"keep"
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        p.verify_file(&file, &cancelled).await,
        Err(fetch_track_cli::Error::Cancelled)
    ));
}

#[tokio::test]
async fn real_ytdlp_download_from_local_http_keeps_native_opus() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let webm = media(temp.path(), "webm", "libopus");
    let bytes = std::fs::read(&webm).unwrap();
    let expected = hash(&webm);
    let app = axum::Router::new().route(
        "/Artist-Title.webm",
        axum::routing::get(move || {
            let bytes = bytes.clone();
            async move { ([(axum::http::header::CONTENT_TYPE, "audio/webm")], bytes) }
        }),
    );
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/Artist-Title.webm", socket.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    let p = pipeline(temp.path());
    let out = temp.path().join("downloads");
    let result = p
        .download(&url, &out, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result.extension().unwrap(), "opus");
    assert_eq!(expected, hash(&result));
    server.abort();
}
