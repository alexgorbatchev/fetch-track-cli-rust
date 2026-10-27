use audio_metadata::{
    CancellationToken, Error,
    cache::Cache,
    process::{Tools, args},
};
use std::time::Duration;
#[tokio::test]
async fn native_process_limits_errors_timeout_cancellation_and_line_delivery() {
    let directory = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let tools = Tools::new(
        std::env::var_os("PATH").unwrap(),
        directory.path().to_owned(),
    );
    let token = CancellationToken::new();
    assert_eq!(
        tools
            .run(
                "printf",
                &args(&["abc"]),
                10,
                Duration::from_secs(1),
                &token
            )
            .await
            .unwrap(),
        b"abc"
    );
    assert!(
        tools
            .run(
                "printf",
                &args(&["abcdef"]),
                2,
                Duration::from_secs(1),
                &token
            )
            .await
            .is_err()
    );
    assert!(
        tools
            .run("false", &args(&[]), 100, Duration::from_secs(1), &token)
            .await
            .is_err()
    );
    assert!(
        tools
            .run(
                "never-installed-executable",
                &args(&[]),
                10,
                Duration::from_secs(1),
                &token
            )
            .await
            .is_err()
    );
    assert!(
        tools
            .run("sleep", &args(&["1"]), 10, Duration::from_millis(5), &token)
            .await
            .is_err()
    );
    let child_token = token.clone();
    let cancellation = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        child_token.cancel();
    });
    assert!(matches!(
        tools
            .run("sleep", &args(&["1"]), 10, Duration::from_secs(2), &token)
            .await,
        Err(Error::Cancelled)
    ));
    cancellation.await.unwrap();
    let token = CancellationToken::new();
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = lines.clone();
    tools
        .lines::<_, _, Error>(
            "sh",
            &args(&["-c", "printf 'out\\n'; printf 'err\\n' >&2"]),
            Duration::from_secs(1),
            &token,
            move |line| {
                store.lock().unwrap().push(line);
                async { Ok(()) }
            },
        )
        .await
        .unwrap();
    let mut got = lines.lock().unwrap().clone();
    got.sort();
    assert_eq!(got, ["err", "out"]);
    assert!(
        tools
            .lines::<_, _, Error>(
                "false",
                &args(&[]),
                Duration::from_secs(1),
                &token,
                |_| async { Ok(()) }
            )
            .await
            .is_err()
    );
    assert!(
        tools
            .lines::<_, _, Error>(
                "sleep",
                &args(&["1"]),
                Duration::from_millis(5),
                &token,
                |_| async { Ok(()) }
            )
            .await
            .is_err()
    );
    assert!(
        tools
            .lines::<_, _, Error>(
                "printf",
                &args(&["x\\n"]),
                Duration::from_secs(1),
                &token,
                |_| async { Err(Error::Invalid("sink failed".into())) }
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn excessive_stderr_fails_immediately_and_kills_descendants() {
    let directory = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let tools = Tools::new(std::env::var_os("PATH").unwrap(), directory.path().into());
    let start = std::time::Instant::now();
    let error = tools
        .run(
            "sh",
            &args(&["-c", "head -c 2097152 /dev/zero >&2; sleep 30"]),
            1024,
            Duration::from_secs(2),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(error.to_string().contains("stderr exceeds"));
}

#[test]
fn cache_expires_corruption_is_miss_disabled_writes_nothing_and_hashes_content() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let cache = Cache::new(temp.path().join("cache"), true);
    assert!(cache.get::<Vec<u8>>("metadata", "key").unwrap().is_none());
    cache
        .put("metadata", "key", &vec![1u8, 2], Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        cache.get::<Vec<u8>>("metadata", "key").unwrap().unwrap(),
        [1, 2]
    );
    cache
        .put("metadata", "expired", &1, Duration::ZERO)
        .unwrap();
    assert!(cache.get::<u8>("metadata", "expired").unwrap().is_none());
    let entries = std::fs::read_dir(temp.path().join("cache/metadata")).unwrap();
    for entry in entries {
        std::fs::write(entry.unwrap().path(), b"broken").unwrap();
    }
    assert!(cache.get::<u8>("metadata", "key").unwrap().is_none());
    let disabled = Cache::new(temp.path().join("disabled"), false);
    disabled.put("x", "y", &1, Duration::from_secs(60)).unwrap();
    assert!(!temp.path().join("disabled").exists());
    assert!(disabled.get::<u8>("x", "y").unwrap().is_none());
    let file = temp.path().join("audio");
    std::fs::write(&file, b"original").unwrap();
    let first = Cache::file_key(&file).unwrap();
    std::fs::write(&file, b"replaced").unwrap();
    assert_ne!(first, Cache::file_key(&file).unwrap());
}
