use cli_command_kit::Mode;
use fetch_track_cli::{CancellationToken, Error, cli};
use std::{io::Write, process::Command};

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fetch-track"));
    command
        .env_remove("FETCH_TRACK_RUST_RELEASE_BASE_URL")
        .env_remove("FETCH_TRACK_RUST_RELEASE_OWNER")
        .env_remove("FETCH_TRACK_RUST_RELEASE_REPO");
    command
}
#[test]
fn help_skill_version_and_offline_validation_contracts() {
    for agent in ["0", "1", "true", "yes"] {
        let out = binary().env("AGENT", agent).arg("skill").output().unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, cli::SKILL.as_bytes());
        assert!(out.stderr.is_empty());
        let out = binary()
            .env("AGENT", agent)
            .arg("--version")
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, b"0.1.0\n");
        for path in [
            vec!["--help"],
            vec!["track", "--help"],
            vec!["track", "verify", "--help"],
            vec!["dependencies", "--help"],
            vec!["dependencies", "install", "--help"],
            vec!["dependencies", "update", "--help"],
            vec!["self", "update", "--help"],
            vec!["help", "track", "verify"],
        ] {
            let out = binary().env("AGENT", agent).args(path).output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let help = String::from_utf8(out.stdout).unwrap();
            if agent != "0" {
                assert!(help.starts_with("ALERT: Agents must read `AGENT=1 fetch-track skill`"));
            } else {
                assert!(!help.starts_with("ALERT:"));
            }
        }
    }
    for args in [
        vec!["skill", "extra"],
        vec!["track", "verify"],
        vec!["query", "--sources", "unknown"],
        vec!["query", "--progress-target", "fd://-1"],
        vec!["dependencies", "install", "unknown"],
        vec!["--unknown"],
    ] {
        let out = binary().args(args).output().unwrap();
        assert!(!out.status.success());
        assert!(!out.stderr.is_empty());
    }
    let out = binary().env("AGENT", "1").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("ALERT:"));
    let out = binary().args(["self", "update"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(err.lines().count(), 1);
    assert!(err.contains("unpublished"));
}

#[tokio::test]
async fn skill_write_failure_and_typed_exit_codes() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "closed",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let matches = cli::command(Mode::Agent)
        .unwrap()
        .try_get_matches_from(["fetch-track", "skill"])
        .unwrap();
    assert!(cli_command_kit::write_skill(&matches, &cli::help_options(), &mut Broken).is_err());
    assert_eq!(cli::exit_code(&Error::Cancelled), 130);
    assert_eq!(
        cli::exit_code(&Error::Dependency(cli_deps::Error::Cancelled)),
        130
    );
    assert_eq!(cli::exit_code(&Error::RadioEdit), 2);
    assert_eq!(cli::exit_code(&Error::NoCandidate), 1);
    cli::execute(&matches, Mode::Human, &CancellationToken::new())
        .await
        .unwrap();
}

#[test]
fn home_folding_obeys_path_segments() {
    let home = std::path::Path::new("/home/alex");
    assert_eq!(
        cli_command_kit::fold_home(std::path::Path::new("/home/alex"), home),
        "~"
    );
    assert_eq!(
        cli_command_kit::fold_home(std::path::Path::new("/home/alex/tracks/a.mp3"), home),
        "~/tracks/a.mp3"
    );
    assert_eq!(
        cli_command_kit::fold_home(std::path::Path::new("/home/alexander/file"), home),
        "/home/alexander/file"
    );
    assert_eq!(
        cli_command_kit::fold_home(std::path::Path::new("/outside/file"), home),
        "/outside/file"
    );
    assert_eq!(
        cli_command_kit::fold_message(
            "file: /home/alex/a; '/home/alex' /home/alexander/a https://example.org/home/alex/a",
            home
        ),
        "file: ~/a; '~' /home/alexander/a https://example.org/home/alex/a"
    );
}

#[tokio::test]
async fn dependency_commands_install_update_and_ensure_through_native_local_release() {
    use axum::{Router, response::Redirect, routing::get};
    use std::sync::Arc;
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    let bytes = b"#!/bin/sh\nprintf '2026.10.01\\n'\n";
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, "yt-dlp", &bytes[..]).unwrap();
    let archive = tar.into_inner().unwrap().finish().unwrap();
    let app = Router::new()
        .route(
            "/owner/tool/releases/latest",
            get(|| async { Redirect::to("/owner/tool/releases/tag/v2026.10.01") }),
        )
        .route(
            "/owner/tool/releases/download/v2026.10.01/{asset}",
            get(move || {
                let archive = archive.clone();
                async move { archive }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let bin = temp.path().join("bin");
    let mut dep = cli_deps::Dependency::new("yt-dlp", "2024.08.01");
    dep.installer = Some(Arc::new(cli_deps::ReleaseInstaller {
        client: cli_deps::ReleaseClient::new(&base).unwrap(),
        owner: "owner".into(),
        repo: "tool".into(),
        binary: "yt-dlp".into(),
    }));
    let manager = cli_deps::Manager::new(cli_deps::Config {
        dependencies: vec![dep],
        bin_dir: Some(bin.clone()),
        runner: Some(Arc::new(cli_deps::ProcessRunner {
            path: Some(bin.as_os_str().to_owned()),
        })),
        ..Default::default()
    })
    .unwrap();
    let token = CancellationToken::new();
    assert!(
        cli::ensure(&manager, false, Mode::Agent, &token)
            .await
            .is_err()
    );
    for args in [
        vec!["dependencies", "check"],
        vec!["dependencies", "install"],
        vec!["dependencies", "update"],
        vec!["dependencies", "install", "yt-dlp"],
        vec!["dependencies", "update", "yt-dlp"],
        vec!["dependencies"],
    ] {
        let matches = cli::command(Mode::Agent)
            .unwrap()
            .try_get_matches_from(std::iter::once("fetch-track").chain(args.iter().copied()))
            .unwrap();
        let (_, sub) = matches.subcommand().unwrap();
        let result = cli::execute_dependencies(&manager, sub, &token).await;
        if args == ["dependencies", "check"] {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
    }
    std::fs::remove_file(bin.join("yt-dlp")).unwrap();
    cli::ensure(&manager, true, Mode::Agent, &token)
        .await
        .unwrap();
    cli::ensure(&manager, false, Mode::Human, &token)
        .await
        .unwrap();
    server.abort();
}
