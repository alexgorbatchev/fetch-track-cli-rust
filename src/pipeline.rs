use crate::{
    CancellationToken, Error, Result,
    candidate::{self, Candidate},
    progress::{Artifact, Event},
    quality::{self, Quality, Structure},
};
use audio_metadata::{
    cache::Cache,
    metadata::Resolver,
    process::{Tools, args},
};
use cli_event_stream::Reporter;
use futures::{StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Probe {
    pub title: String,
    pub artist: String,
    pub duration: f64,
    pub format: String,
    pub date: String,
    pub comment: String,
    pub codec: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Verification {
    pub metadata: Probe,
    pub structure: Structure,
    pub quality: Quality,
    pub status: String,
}
pub struct Options {
    pub out_dir: PathBuf,
    pub sources: Vec<String>,
    pub skip_verify: bool,
    pub skip_metadata: bool,
    pub interactive: bool,
    pub agent: bool,
    pub verbose: bool,
}
pub struct Pipeline {
    pub tools: Tools,
    pub cache: Cache,
    pub resolver: Resolver,
    pub reporter: Reporter,
}
fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}
mod media;
mod sources;
impl Pipeline {
    pub async fn acquire(
        &self,
        input: &str,
        options: &Options,
        token: &CancellationToken,
    ) -> Result<Artifact> {
        let human = crate::progress::HumanProgress::new(options.agent, options.verbose);
        let mut target = input.to_owned();
        let (raw_artist, raw_title) = crate::split_query(input);
        let mut artist = raw_artist.to_owned();
        let mut title = raw_title.to_owned();
        let mut direct = None;
        if let Some(url) = crate::source_url(input) {
            target = url.to_string();
            match self.probe_url(&target, token).await {
                Ok(probe) => {
                    let (a, t) = crate::split_query(&probe.title);
                    artist = if a.is_empty() {
                        probe.artist.clone()
                    } else {
                        a.into()
                    };
                    title = t.into();
                    direct = Some(Candidate {
                        id: "direct_url".into(),
                        title: probe.title,
                        duration: probe.duration,
                        source: "direct_url".into(),
                        url: target.clone(),
                        score: 0,
                    });
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => self.notice(&format!("URL metadata: {e}"), options)?,
            }
        }
        self.phase("search", 2, options, &human, token).await?;
        let mut candidates = match self
            .search(&options.sources, &title, &artist, &title, token)
            .await
        {
            Ok(v) => v,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => {
                if direct.is_none() && crate::source_url(input).is_none() {
                    return Err(e);
                }
                self.notice(&format!("search: {e}"), options)?;
                Vec::new()
            }
        };
        if let Some(c) = direct {
            let mut e = Event::new("candidate_found", "search");
            e.candidate = Some((&c).into());
            self.reporter.emit(e, token).await?;
            candidates.push(c);
        }
        candidates = candidate::rank(candidates, &artist, &title);
        if options.verbose {
            for c in &candidates {
                eprintln!(
                    "candidate: {} [{} {:.0}s] score={}",
                    cli_command_kit::display_message(&c.title),
                    c.source,
                    c.duration,
                    c.score
                );
            }
        }
        if !candidates.is_empty() {
            let index = if options.interactive && !options.agent {
                if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    return Err(Error::Invalid(
                        "interactive selection requires a terminal".into(),
                    ));
                }
                let labels: Vec<_> = candidates
                    .iter()
                    .map(|c| {
                        format!(
                            "{} [{} {:.0}s] score={}",
                            c.title, c.source, c.duration, c.score
                        )
                    })
                    .collect();
                dialoguer::Select::new()
                    .with_prompt("Select track")
                    .items(&labels)
                    .default(0)
                    .interact_opt()
                    .map_err(|e| Error::Invalid(e.to_string()))?
                    .ok_or(Error::Cancelled)?
            } else {
                0
            };
            let selected = &candidates[index];
            target = selected.url.clone();
            let mut e = Event::new("candidate_selected", "search");
            e.candidate = Some(selected.into());
            self.reporter.emit(e, token).await?;
        }
        std::fs::create_dir_all(&options.out_dir)?;
        let tmp = Temporary::new(&options.out_dir)?;
        self.phase("download", 3, options, &human, token).await?;
        let staged_download = self.download(&target, tmp.path(), token).await?;
        let downloaded = audio_metadata::tag::clean_output(&staged_download, &options.out_dir)?;
        let mut result = Artifact {
            ..Default::default()
        };
        if !options.skip_verify {
            self.phase("verify", 4, options, &human, token).await?;
            match self.verify_file(&downloaded, token).await {
                Ok(v) => {
                    result.duration_seconds = v.metadata.duration;
                    result.bandwidth_hz = v.quality.bandwidth_hz;
                    result.bandwidth_rating = v.quality.bandwidth_rating.clone();
                    result.suggested_gain_db = v.quality.suggested_gain_db;
                    result.status = v.status.clone();
                    if options.verbose {
                        eprintln!(
                            "quality: {} ({} Hz); peak={:.2} dBFS rms={:.2} dBFS",
                            v.quality.bandwidth_rating,
                            v.quality.bandwidth_hz,
                            v.quality.peak_dbfs,
                            v.quality.rms_dbfs
                        );
                    }
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => self.notice(&format!("verification: {e}"), options)?,
            }
        }
        let final_path = if !options.skip_metadata {
            self.phase("metadata", 5, options, &human, token).await?;
            let raw = downloaded.file_stem().unwrap_or_default().to_string_lossy();
            let (a, t) = crate::split_query(&raw);
            let mut metadata = self
                .resolver
                .resolve(&downloaded, &raw, a, t, token)
                .await?;
            metadata.audio_source_url = target;
            if options.verbose {
                eprintln!(
                    "metadata: {} | {} - {} | {}",
                    metadata.provider,
                    cli_command_kit::display_message(&metadata.artist),
                    cli_command_kit::display_message(&metadata.title),
                    metadata.release_date
                );
            }
            metadata.fetched_at = chrono::Utc::now();
            let art = if metadata.cover_url.is_empty() {
                None
            } else {
                match self.resolver.artwork(&metadata.cover_url, token).await {
                    Ok(v) => Some(v),
                    Err(audio_metadata::Error::Cancelled) => return Err(Error::Cancelled),
                    Err(e) => {
                        self.notice(&format!("artwork: {e}"), options)?;
                        None
                    }
                }
            };
            result.title = metadata.title.clone();
            result.artist = metadata.artist.clone();
            result.album = metadata.album.clone();
            result.release_year = metadata.release_year();
            match audio_metadata::tag::apply(
                &downloaded,
                &metadata,
                &options.out_dir,
                art.as_deref(),
                token,
            ) {
                Ok(v) => v,
                Err(audio_metadata::Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => {
                    self.notice(&format!("tagging: {e}"), options)?;
                    audio_metadata::tag::clean_output(&downloaded, &options.out_dir)?
                }
            }
        } else {
            audio_metadata::tag::clean_output(&downloaded, &options.out_dir)?
        };
        result.path = crate::display(&final_path);
        let mut event = Event::new("complete", "complete");
        event.step = Some(5);
        event.total_steps = Some(5);
        event.result = Some(result.clone());
        self.reporter.emit(event, token).await?;
        Ok(result)
    }
    async fn phase(
        &self,
        phase: &str,
        step: u8,
        options: &Options,
        human: &crate::progress::HumanProgress,
        token: &CancellationToken,
    ) -> Result<()> {
        self.reporter.emit(Event::phase(phase, step), token).await?;
        human.phase(phase);
        if options.verbose {
            eprintln!("{phase}...");
        }
        Ok(())
    }
    fn notice(&self, message: &str, _options: &Options) -> Result<()> {
        use std::io::Write;
        writeln!(
            std::io::stderr(),
            "WARN: {}",
            cli_command_kit::display_message(message)
        )?;
        Ok(())
    }
}

pub struct Temporary {
    directory: Option<tempfile::TempDir>,
    root: PathBuf,
    created_root: bool,
}
impl Temporary {
    pub fn new(parent: &Path) -> Result<Self> {
        let root = parent.join(".tmp");
        let created_root = match std::fs::create_dir(&root) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(e.into()),
        };
        let directory = tempfile::Builder::new()
            .prefix("fetch-track-")
            .tempdir_in(&root)?;
        Ok(Self {
            directory: Some(directory),
            root,
            created_root,
        })
    }
    pub fn path(&self) -> &Path {
        self.directory
            .as_ref()
            .expect("temporary directory alive")
            .path()
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        drop(self.directory.take());
        if self.created_root {
            let _ = std::fs::remove_dir(&self.root);
        }
    }
}
