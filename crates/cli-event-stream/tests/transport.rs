use cli_event_stream::{CancellationToken, Error, Reporter, Target};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
struct Event {
    kind: String,
    phase: String,
    message: String,
    percent: Option<f64>,
}
impl Event {
    fn new(kind: &str, phase: &str) -> Self {
        Self {
            kind: kind.into(),
            phase: phase.into(),
            message: String::new(),
            percent: None,
        }
    }
}
use std::time::Duration;
use tokio::io::AsyncReadExt;

#[test]
fn inherited_fd_backpressure_cancels_without_hanging_runtime() {
    use std::os::fd::FromRawFd;
    let mut fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let _reader = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    let writer = unsafe { std::fs::File::from_raw_fd(fds[1]) };
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fd_backpressure_child", "--nocapture"])
        .env("FETCH_TRACK_TEST_PROGRESS_FD", fds[1].to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    drop(writer);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("cancelled inherited FD write kept Tokio runtime alive");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[tokio::test]
async fn fd_backpressure_child() {
    let Ok(raw) = std::env::var("FETCH_TRACK_TEST_PROGRESS_FD") else {
        return;
    };
    let token = CancellationToken::new();
    let raw = raw.parse().unwrap();
    let original_flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
    let reporter = Reporter::connect(&Target::Fd(raw), &token).await.unwrap();
    let cancel = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.cancel();
    });
    let mut event = Event::new("progress", "download");
    event.message = "x".repeat(1024 * 1024);
    assert!(matches!(
        reporter.emit(event, &token).await,
        Err(Error::Cancelled)
    ));
    drop(reporter);
    assert_eq!(unsafe { libc::fcntl(raw, libc::F_GETFL) }, original_flags);
}

#[tokio::test]
async fn descriptor_failure_restores_flags_and_regular_file_keeps_caller_open() {
    use std::os::fd::AsRawFd;
    let token = CancellationToken::new();
    let (writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
    let flags = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) };
    let reporter = Reporter::connect(&Target::Fd(writer.as_raw_fd()), &token)
        .await
        .unwrap();
    let retained = reporter.clone();
    reader.shutdown(std::net::Shutdown::Both).unwrap();
    drop(reader);
    assert!(
        reporter
            .emit(Event::new("complete", "broken"), &token)
            .await
            .is_err()
    );
    drop(reporter);
    assert_ne!(
        unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) } & libc::O_NONBLOCK,
        0
    );
    drop(retained);
    assert_eq!(
        unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) },
        flags
    );
    let mut file = tempfile::tempfile_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let reporter = Reporter::connect(&Target::Fd(file.as_raw_fd()), &token)
        .await
        .unwrap();
    reporter
        .emit(Event::new("complete", "file"), &token)
        .await
        .unwrap();
    drop(reporter);
    std::io::Write::write_all(&mut file, b"caller-owned\n").unwrap();
    use std::io::{Read, Seek};
    file.rewind().unwrap();
    let mut output = String::new();
    file.read_to_string(&mut output).unwrap();
    assert!(output.contains("\"phase\":\"file\""));
    assert!(output.ends_with("caller-owned\n"));
}

#[tokio::test]
async fn ndjson_is_atomic_across_concurrent_emitters_and_reports_broken_streams() {
    let (writer, mut reader) = tokio::io::duplex(65536);
    let reporter = Reporter::writer(writer);
    let token = CancellationToken::new();
    let mut tasks = Vec::new();
    for i in 0..32 {
        let reporter = reporter.clone();
        let token = token.clone();
        tasks.push(tokio::spawn(async move {
            let mut event = Event::new("progress", "download");
            event.percent = Some(i as f64);
            reporter.emit(event, &token).await.unwrap()
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    drop(reporter);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    let mut values = Vec::new();
    for line in bytes.split(|b| *b == b'\n').filter(|b| !b.is_empty()) {
        let event: Event = serde_json::from_slice(line).unwrap();
        values.push(event.percent.unwrap() as u64);
    }
    values.sort();
    assert_eq!(values, (0..32).collect::<Vec<_>>());
    let (writer, reader) = tokio::io::duplex(1);
    drop(reader);
    assert!(
        Reporter::writer(writer)
            .emit(Event::new("error", "test"), &token)
            .await
            .is_err()
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let (writer, _reader) = tokio::io::duplex(1);
    assert!(matches!(
        Reporter::writer(writer)
            .emit(Event::new("complete", "test"), &cancelled)
            .await,
        Err(Error::Cancelled)
    ));
    Reporter::default()
        .emit(Event::new("complete", "test"), &token)
        .await
        .unwrap();
}

#[tokio::test]
async fn ipc_connects_unix_tcp_and_duplicates_borrowed_fd() {
    use std::os::fd::AsRawFd;
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let path = temp.path().join("progress.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let token = CancellationToken::new();
    let reporter = Reporter::connect(&Target::Unix(path.to_owned()), &token)
        .await
        .unwrap();
    let (mut stream, _) = listener.accept().await.unwrap();
    reporter
        .emit(Event::new("complete", "done"), &token)
        .await
        .unwrap();
    drop(reporter);
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Event>(&bytes).unwrap().kind,
        "complete"
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let reporter = Reporter::connect(
        &Target::Tcp(listener.local_addr().unwrap().to_string()),
        &token,
    )
    .await
    .unwrap();
    let (mut stream, _) = listener.accept().await.unwrap();
    reporter
        .emit(Event::new("error", "download"), &token)
        .await
        .unwrap();
    drop(reporter);
    bytes.clear();
    stream.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Event>(&bytes).unwrap().kind,
        "error"
    );
    let (file, mut reader) = std::os::unix::net::UnixStream::pair().unwrap();
    let before = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    let reporter = Reporter::connect(&Target::Fd(file.as_raw_fd()), &token)
        .await
        .unwrap();
    reporter
        .emit(Event::new("complete", "fd"), &token)
        .await
        .unwrap();
    drop(reporter);
    assert_eq!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) },
        before
    );
    std::io::Write::write_all(&mut &file, b"caller-still-open\n").unwrap();
    drop(file);
    let mut result = String::new();
    std::io::Read::read_to_string(&mut reader, &mut result).unwrap();
    assert!(result.ends_with("caller-still-open\n"));
    assert!(
        Reporter::connect(&Target::Fd(i32::MAX), &token)
            .await
            .is_err()
    );
    assert!(
        Reporter::connect(&Target::Unix("/nonexistent/progress.sock".into()), &token)
            .await
            .is_err()
    );
    for invalid in [
        "",
        "garbage",
        "fd://-1",
        "fd://2147483648",
        "https://example.org",
        "unix://",
        "tcp://",
    ] {
        assert!(Target::parse(invalid).is_err(), "{invalid}");
    }
    for (raw, expected) in [
        ("stdout://", Target::Stdout),
        ("stderr", Target::Stderr),
        ("fd://3", Target::Fd(3)),
        ("unix://relative.sock", Target::Unix("relative.sock".into())),
        ("./foo", Target::Unix("./foo".into())),
        ("127.0.0.1:8080", Target::Tcp("127.0.0.1:8080".into())),
    ] {
        assert_eq!(Target::parse(raw).unwrap(), expected);
    }
}
