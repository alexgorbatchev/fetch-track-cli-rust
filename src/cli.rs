use crate::{
    CancellationToken, Error, Result,
    pipeline::{Options, Pipeline},
    progress::Event,
};
use audio_metadata::{
    cache::Cache,
    metadata::{Endpoints, Resolver},
    process::Tools,
};
use clap::{Arg, ArgAction, ArgMatches, Command};
use cli_command_kit::{HelpOptions, Mode};
use cli_event_stream::{Reporter, Target};
use std::{
    io::{IsTerminal, Write},
    path::PathBuf,
    sync::Arc,
};

pub const SKILL: &str = "Install the fetch-track skill using the Skills CLI:\n\n  npx skills add alexgorbatchev/fetch-track-cli --skill fetch-track\n\nSkill:\n\n  https://github.com/alexgorbatchev/fetch-track-cli/tree/main/skills/fetch-track\n";
pub fn help_options() -> HelpOptions {
    HelpOptions {
        skill: Some(SKILL.into()),
        ..Default::default()
    }
}
fn boolean(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .help(help)
        .action(ArgAction::SetTrue)
}
pub fn command(mode: Mode) -> Result<Command> {
    let root = Command::new("fetch-track")
        .about("Find, download, verify, and tag native audio tracks")
        .disable_version_flag(true)
        .arg(boolean("version", "Print the raw version").global(true))
        .arg(
            boolean("verbose", "Show detailed progress")
                .short('v')
                .global(true),
        )
        .arg(Arg::new("query").value_name("QUERY").num_args(0..))
        .arg(
            Arg::new("out-dir")
                .long("out-dir")
                .short('o')
                .default_value(".")
                .value_name("DIRECTORY")
                .help("Save tracks in this directory"),
        )
        .arg(
            Arg::new("sources")
                .long("sources")
                .short('s')
                .default_value("youtube,soundcloud")
                .value_name("SOURCES")
                .value_parser(parse_sources)
                .help("Comma-separated search sources: youtube,soundcloud"),
        )
        .arg(boolean("skip-verify", "Skip audio spectrum inspection"))
        .arg(boolean("skip-metadata", "Skip metadata lookup and tagging"))
        .arg(boolean("interactive", "Choose among search candidates").short('i'))
        .arg(boolean("no-cache", "Bypass cached search and metadata"))
        .arg(
            boolean(
                "auto-install",
                "Install missing external tools without prompting",
            )
            .global(true),
        )
        .arg(
            Arg::new("progress-target")
                .long("progress-target")
                .value_name("URI")
                .help("Stream NDJSON to unix://, tcp://, fd://, stdout, or stderr")
                .value_parser(parse_target),
        )
        .arg(
            Arg::new("progress-socket")
                .long("progress-socket")
                .value_name("PATH")
                .help("Stream NDJSON to a Unix socket"),
        )
        .subcommand(
            Command::new("track")
                .about("Inspect audio tracks")
                .subcommand_required(true)
                .subcommand(
                    Command::new("verify")
                        .about("Inspect a file or URL for mix structure and audio quality")
                        .arg(
                            Arg::new("target")
                                .value_name("TARGET")
                                .num_args(1..)
                                .required(true),
                        ),
                ),
        )
        .subcommand(
            Command::new("dependencies")
                .about("Manage required external tools")
                .subcommand(Command::new("check").about("Check installed and required versions"))
                .subcommand(
                    Command::new("install")
                        .about("Install named or missing tools")
                        .arg(
                            Arg::new("names")
                                .value_name("DEPENDENCY")
                                .num_args(0..)
                                .value_parser(["yt-dlp", "ffmpeg", "ffprobe"]),
                        ),
                )
                .subcommand(
                    Command::new("update")
                        .about("Update named tools or all tools")
                        .arg(
                            Arg::new("names")
                                .value_name("DEPENDENCY")
                                .num_args(0..)
                                .value_parser(["yt-dlp", "ffmpeg", "ffprobe"]),
                        ),
                ),
        )
        .subcommand(
            Command::new("self")
                .about("Manage this executable")
                .subcommand_required(true)
                .subcommand(Command::new("update").about("Install a published Rust release")),
        );
    cli_command_kit::setup(root, mode, &help_options()).map_err(|e| Error::Invalid(e.to_string()))
}
fn parse_sources(raw: &str) -> std::result::Result<String, String> {
    if raw
        .split(',')
        .all(|s| matches!(s.trim(), "youtube" | "soundcloud"))
    {
        Ok(raw.into())
    } else {
        Err("sources must contain youtube and/or soundcloud; use a direct URL for Bandcamp".into())
    }
}
fn parse_target(raw: &str) -> std::result::Result<String, String> {
    Target::parse(raw)
        .map(|_| raw.to_owned())
        .map_err(|e| e.to_string())
}

pub fn dependency_manager() -> Result<cli_deps::Manager> {
    let bin = cli_deps::managed_bin_dir("fetch-track")?;
    let runner = Arc::new(cli_deps::ProcessRunner {
        path: Some(cli_deps::managed_path(
            &bin,
            std::env::var_os("PATH").as_deref(),
        )?),
    });
    let mut yt = cli_deps::Dependency::new("yt-dlp", "2024.08.01");
    yt.install_url = "https://github.com/yt-dlp/yt-dlp#installation".into();
    yt.installer = Some(Arc::new(cli_deps::YtDlpInstaller {
        client: cli_deps::ReleaseClient::github()?,
        runner: runner.clone(),
    }));
    let mut ffmpeg = cli_deps::Dependency::new("ffmpeg", "4.4");
    ffmpeg.install_url = "https://ffmpeg.org/download.html".into();
    ffmpeg.version_args = vec!["-version".into()];
    ffmpeg.installer = Some(Arc::new(cli_deps::PackageInstaller {
        package: "ffmpeg".into(),
        runner: runner.clone(),
    }));
    let mut ffprobe = cli_deps::Dependency::new("ffprobe", "4.4");
    ffprobe.install_url = ffmpeg.install_url.clone();
    ffprobe.version_args = ffmpeg.version_args.clone();
    ffprobe.installer = ffmpeg.installer.clone();
    ffprobe.parse_version = Some(Arc::new(|_, output| {
        cli_deps::default_version_parser("ffmpeg", &output.replacen("ffprobe", "ffmpeg", 1))
    }));
    Ok(cli_deps::Manager::new(cli_deps::Config {
        app_name: "fetch-track".into(),
        dependencies: vec![yt, ffmpeg, ffprobe],
        runner: Some(runner),
        ..Default::default()
    })?)
}

pub async fn ensure(
    manager: &cli_deps::Manager,
    auto: bool,
    mode: Mode,
    token: &CancellationToken,
) -> Result<()> {
    let reports = manager.verify(token).await?;
    let failed: Vec<_> = reports.iter().filter(|r| !r.satisfied).collect();
    if failed.is_empty() {
        return Ok(());
    }
    if auto {
        manager.install_unsatisfied(token).await?.into_result()?;
        return Ok(());
    }
    if mode == Mode::Human && std::io::stdin().is_terminal() {
        let accepted = dialoguer::Confirm::new()
            .with_prompt("Install missing or outdated audio tools?")
            .default(false)
            .interact()
            .map_err(|e| Error::Invalid(e.to_string()))?;
        if accepted {
            manager.install_unsatisfied(token).await?.into_result()?;
            return Ok(());
        }
    }
    Err(Error::Invalid(format!(
        "required dependencies: {}. Run fetch-track dependencies install or pass --auto-install",
        failed
            .iter()
            .map(|r| format!(
                "{} (installed {}, required {} or newer)",
                r.name,
                if r.detected_version.is_empty() {
                    "missing"
                } else {
                    &r.detected_version
                },
                r.min_version
            ))
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

pub async fn execute(matches: &ArgMatches, mode: Mode, token: &CancellationToken) -> Result<()> {
    if matches.get_flag("version") {
        return Ok(writeln!(
            std::io::stdout(),
            "{}",
            env!("CARGO_PKG_VERSION")
        )?);
    }
    if cli_command_kit::write_skill(matches, &help_options(), &mut std::io::stdout())? {
        return Ok(());
    }
    if matches.subcommand_name() == Some("self") {
        let update = crate::update::from_env()?
            .update_running(env!("CARGO_PKG_VERSION"), token)
            .await?;
        println!(
            "OK: {}",
            match update {
                cli_deps::Update::Current(v) => format!("already current ({v})"),
                cli_deps::Update::Installed(v) => format!("updated to {v}"),
            }
        );
        return Ok(());
    }
    if matches.subcommand().is_none() && matches.get_many::<String>("query").is_none() {
        command(mode)?.print_help()?;
        return Ok(());
    }
    let target = matches
        .get_one::<String>("progress-target")
        .cloned()
        .or_else(|| {
            matches
                .get_one::<String>("progress-socket")
                .map(|p| format!("unix://{p}"))
        })
        .or_else(|| {
            std::env::var("FETCH_TRACK_PROGRESS_TARGET")
                .ok()
                .filter(|s| !s.is_empty())
        });
    let target = target.as_deref().map(Target::parse).transpose()?;
    let manager = dependency_manager()?;
    if let Some(("dependencies", sub)) = matches.subcommand() {
        return execute_dependencies(&manager, sub, token).await;
    }
    ensure(&manager, matches.get_flag("auto-install"), mode, token).await?;
    execute_pipeline(matches, mode, token, &manager, target).await
}

pub async fn execute_dependencies(
    manager: &cli_deps::Manager,
    sub: &ArgMatches,
    token: &CancellationToken,
) -> Result<()> {
    match sub.subcommand() {
        None | Some(("check", _)) => {
            let reports = manager.verify(token).await?;
            for report in &reports {
                println!(
                    "{}: {} (installed {}, required {})",
                    report.name,
                    if report.satisfied { "OK" } else { "FAIL" },
                    report.detected_version,
                    report.min_version
                );
            }
            if reports.iter().any(|r| !r.satisfied) {
                return Err(Error::Invalid(
                    "required dependencies are missing or outdated".into(),
                ));
            }
        }
        Some((verb, args)) => {
            let names = args
                .get_many::<String>("names")
                .map(|v| v.cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            if names.is_empty() {
                let outcome = if verb == "install" {
                    manager.install_unsatisfied(token).await?
                } else {
                    manager.update_all(token).await?
                };
                outcome.into_result()?;
            } else {
                for name in names {
                    if verb == "install" {
                        manager.install(&name, token).await?;
                    } else {
                        manager.update(&name, token).await?;
                    }
                }
            }
            println!("OK: dependencies {verb} complete");
        }
    }
    Ok(())
}

async fn execute_pipeline(
    matches: &ArgMatches,
    mode: Mode,
    token: &CancellationToken,
    manager: &cli_deps::Manager,
    target: Option<Target>,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let tools = Tools::new(
        cli_deps::managed_path(manager.bin_dir(), std::env::var_os("PATH").as_deref())?,
        cwd,
    );
    let cache_root = dirs::cache_dir()
        .ok_or_else(|| Error::Invalid("cannot resolve cache directory".into()))?
        .join("fetch-track");
    let cache = Cache::new(
        cache_root,
        !matches
            .try_get_one::<bool>("no-cache")
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false),
    );
    let resolver = Resolver::new(
        tools.clone(),
        cache.clone(),
        Endpoints::default(),
        "fetch-track/0.1.0 (https://github.com/alexgorbatchev/fetch-track-cli)",
    )?;
    let reporter = if let Some(target) = &target {
        Reporter::connect(target, token).await?
    } else {
        Reporter::default()
    };
    let pipeline = Pipeline {
        tools,
        cache,
        resolver,
        reporter: reporter.clone(),
    };
    if let Some(("track", sub)) = matches.subcommand() {
        let (_, sub) = sub.subcommand().expect("required verb");
        let target = sub
            .get_many::<String>("target")
            .expect("required target")
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        let human =
            crate::progress::HumanProgress::new(mode == Mode::Agent, matches.get_flag("verbose"));
        human.phase("Verifying audio");
        let report = pipeline.verify(&target, token).await?;
        drop(human);
        if mode == Mode::Agent {
            serde_json::to_writer(std::io::stdout(), &report)?;
            println!();
        } else {
            println!(
                "{}\nDuration: {:.0}s ({})\nBandwidth: {} ({} Hz)\nPeak: {:.2} dBFS; RMS: {:.2} dBFS; gain: {:+.1} dB\nStatus: {}",
                report.metadata.title,
                report.metadata.duration,
                report.structure.description,
                report.quality.bandwidth_rating,
                report.quality.bandwidth_hz,
                report.quality.peak_dbfs,
                report.quality.rms_dbfs,
                report.quality.suggested_gain_db,
                report.status
            );
        }
        if report.structure.radio_edit {
            return Err(Error::RadioEdit);
        }
        return Ok(());
    }
    let query = matches
        .get_many::<String>("query")
        .expect("query checked")
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let options = Options {
        out_dir: PathBuf::from(
            matches
                .get_one::<String>("out-dir")
                .expect("default directory"),
        ),
        sources: matches
            .get_one::<String>("sources")
            .expect("default sources")
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect(),
        skip_verify: matches.get_flag("skip-verify"),
        skip_metadata: matches.get_flag("skip-metadata"),
        interactive: matches.get_flag("interactive"),
        agent: mode == Mode::Agent,
        verbose: matches.get_flag("verbose"),
    };
    match pipeline.acquire(&query, &options, token).await {
        Ok(result) => {
            if target != Some(Target::Stdout) {
                if mode == Mode::Agent {
                    println!("output: {}\nstatus: {}", result.path, result.status);
                } else {
                    println!("DONE: {}", result.path);
                }
            }
            Ok(())
        }
        Err(error) => {
            let mut event = Event::new("error", "pipeline");
            event.error = cli_command_kit::display_message(&error.to_string());
            if !token.is_cancelled() {
                reporter.emit(event, token).await?;
            }
            Err(error)
        }
    }
}
pub fn exit_code(error: &Error) -> u8 {
    match error {
        Error::Cancelled | Error::Dependency(cli_deps::Error::Cancelled) => 130,
        Error::RadioEdit => 2,
        _ => 1,
    }
}
