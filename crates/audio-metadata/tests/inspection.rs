use audio_metadata::{CancellationToken, Error, inspect, process::Tools};

#[tokio::test]
async fn native_opus_stream_tags_and_audio_properties_are_visible() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(root).unwrap();
    let path = dir.path().join("native.opus");
    let child = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=sample_rate=48000:duration=1",
            "-c:a",
            "libopus",
            "-metadata",
            "title=Song",
            "-metadata",
            "artist=Artist",
            "-metadata",
            "album=Album",
            "-metadata",
            "date=1997-09-22",
            "-metadata",
            "BPM=127.5",
            "-metadata",
            "composer=Composer",
            "-metadata",
            "publisher=Label",
            "-metadata",
            "remixer=Remixer",
            "-y",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(child.status.success());
    let tools = Tools::new(std::env::var_os("PATH").unwrap(), dir.path().to_owned());
    let info = inspect::inspect(&tools, &path, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(info.path, path);
    assert_eq!(
        (
            info.title.as_str(),
            info.artist.as_str(),
            info.album.as_str()
        ),
        ("Song", "Artist", "Album")
    );
    assert_eq!(info.date, "1997-09-22");
    assert_eq!(info.year, Some(1997));
    assert_eq!(info.bpm, Some(127.5));
    assert_eq!(info.label, "Label");
    assert_eq!(info.composer, "Composer");
    assert_eq!(info.remixer, "Remixer");
    assert_eq!(info.tags["bpm"], "127.5");
    assert_eq!(info.codec, "opus");
    assert_eq!(info.sample_rate_hz, 48000);
    assert_eq!(info.channels, 1);
    assert!(info.duration_seconds >= 1.0 && info.duration_seconds < 1.1);
    assert!(info.bit_rate_kbps > 0);
    assert!(!info.has_artwork);
    use std::os::unix::ffi::OsStringExt;
    let native = dir
        .path()
        .join(std::ffi::OsString::from_vec(b"native-\xff.opus".to_vec()));
    std::fs::rename(&path, &native).unwrap();
    let actual = inspect::inspect(&tools, &native, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(actual.path, native);
    assert!(
        inspect::inspect(
            &tools,
            &dir.path().join("missing"),
            &CancellationToken::new()
        )
        .await
        .is_err()
    );
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        inspect::inspect(&tools, &native, &token).await,
        Err(Error::Cancelled)
    ));
}
