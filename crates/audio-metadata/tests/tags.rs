use audio_metadata::{CancellationToken, metadata::Metadata, tag};
use lofty::{file::TaggedFileExt, tag::Accessor};
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
#[tokio::test]
async fn tags_preserve_encoded_audio_all_native_formats_and_normalize_cover() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let out = temp.path().join("output");
    let image = image::DynamicImage::new_rgb8(80, 40);
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let metadata = Metadata {
        title: "Jóga [ABC]".into(),
        artist: "Björk".into(),
        album: "Homogenic".into(),
        release_date: "1997-09-22".into(),
        provider: "local fixture".into(),
        audio_source_url: "https://example.org/audio".into(),
        fetched_at: "2026-10-14T00:00:00Z".parse().unwrap(),
        ..Default::default()
    };
    for (ext, codec) in [
        ("mp3", "libmp3lame"),
        ("m4a", "aac"),
        ("opus", "libopus"),
        ("flac", "flac"),
        ("ogg", "libvorbis"),
    ] {
        let source = media(temp.path(), ext, codec);
        let original = std::fs::read(&source).unwrap();
        let before = hash(&source);
        let target = tag::apply(
            &source,
            &metadata,
            &out,
            Some(png.get_ref()),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(before, hash(&target), "{ext} audio packets changed");
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(
            target.file_name().unwrap().to_string_lossy(),
            format!("Björk - Jóga.{ext}")
        );
        let file = lofty::read_from_path(&target).unwrap();
        let tag = file.primary_tag().unwrap();
        assert_eq!(tag.title().unwrap(), "Jóga [ABC]");
        assert_eq!(tag.album().unwrap(), "Homogenic");
        assert_eq!(
            tag.get_string(lofty::tag::ItemKey::RecordingDate),
            Some("1997-09-22"),
            "{ext} date precision"
        );
        assert!(tag.comment().unwrap().contains("Fetched: 2026-10-14"));
        let picture = &tag.pictures()[0];
        let art = image::load_from_memory(picture.data()).unwrap();
        assert_eq!((art.width(), art.height()), (1400, 1400));
        let tools = audio_metadata::process::Tools::new(
            std::env::var_os("PATH").unwrap(),
            temp.path().to_owned(),
        );
        let inspection =
            audio_metadata::inspect::inspect(&tools, &target, &CancellationToken::new())
                .await
                .unwrap();
        assert!(
            inspection.has_artwork && inspection.is_square_artwork,
            "{ext} artwork missing"
        );
        assert_eq!(
            (inspection.artwork_width, inspection.artwork_height),
            (1400, 1400)
        );
        assert_eq!(inspection.title, metadata.title);
        let existing = std::fs::read(&target).unwrap();
        assert!(tag::apply(&source, &metadata, &out, None, &CancellationToken::new()).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), existing);
        // Updating exactly the same master is atomic and still preserves audio bytes.
        tag::apply(&target, &metadata, &out, None, &CancellationToken::new()).unwrap();
        assert_eq!(before, hash(&target));
    }
    assert!(tag::square_art(b"invalid image").is_err());
    assert_eq!(tag::clean_filename(" ../A:[id] B/?\n "), "A B");
    assert_eq!(tag::clean_filename("[id]..."), "Unknown Track");
}

#[test]
fn clean_output_preserves_bytes_keeps_sources_and_rejects_collisions() {
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let source = dir.path().join("Artist - Song [video-id].flac");
    std::fs::write(&source, b"encoded audio fixture").unwrap();
    let target = tag::clean_output(&source, dir.path()).unwrap();
    assert_eq!(target.file_name().unwrap(), "Artist - Song.flac");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        std::fs::read(&source).unwrap()
    );
    assert!(tag::clean_output(&source, dir.path()).is_err());
    assert_eq!(tag::clean_output(&target, dir.path()).unwrap(), target);
    let no_extension = dir.path().join("no-extension");
    std::fs::write(&no_extension, b"unidentified source").unwrap();
    assert!(
        tag::write(
            &no_extension,
            &dir.path().join("destination"),
            &tag::TagUpdate::default(),
            &CancellationToken::new()
        )
        .is_err()
    );
    let target = tag::clean_output(&no_extension, dir.path()).unwrap();
    assert_eq!(target.file_name().unwrap(), "no-extension.audio");
    assert_eq!(std::fs::read(target).unwrap(), b"unidentified source");
}
