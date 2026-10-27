use audio_metadata::{
    CancellationToken, Error,
    tag::{self, TagUpdate},
};
use lofty::{
    file::TaggedFileExt,
    tag::{Accessor, ItemKey},
};

#[test]
fn partial_tag_updates_preserve_unspecified_fields_and_audio_and_fail_atomically() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(root).unwrap();
    let file = dir.path().join("source.flac");
    let child = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=1",
            "-metadata",
            "title=Original",
            "-metadata",
            "artist=Artist",
            "-metadata",
            "album=Album",
            "-metadata",
            "comment=Keep comment",
            "-c:a",
            "flac",
            "-y",
        ])
        .arg(&file)
        .output()
        .unwrap();
    assert!(child.status.success());
    let original = std::fs::read(&file).unwrap();
    let update = TagUpdate {
        genre: Some("House".into()),
        comment: Some("Explicit comment".into()),
        date: Some("2000-02-29".into()),
        ..Default::default()
    };
    let target = dir.path().join("target.flac");
    tag::write(&file, &target, &update, &CancellationToken::new()).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), original);
    let tagged = lofty::read_from_path(&target).unwrap();
    let actual = tagged.primary_tag().unwrap();
    assert_eq!(actual.title().as_deref(), Some("Original"));
    assert_eq!(actual.artist().as_deref(), Some("Artist"));
    assert_eq!(actual.album().as_deref(), Some("Album"));
    assert_eq!(actual.genre().as_deref(), Some("House"));
    assert_eq!(actual.comment().as_deref(), Some("Explicit comment"));
    assert_eq!(
        actual.get_string(ItemKey::RecordingDate),
        Some("2000-02-29")
    );
    let before = std::fs::read(&target).unwrap();
    let invalid = TagUpdate {
        artwork: Some(b"not image".to_vec()),
        ..Default::default()
    };
    assert!(tag::write(&target, &target, &invalid, &CancellationToken::new()).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), before);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        tag::write(&target, &target, &update, &cancelled),
        Err(Error::Cancelled)
    ));
    assert_eq!(std::fs::read(&target).unwrap(), before);
}
