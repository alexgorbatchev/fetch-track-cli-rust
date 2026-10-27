use super::*;
impl Pipeline {
    pub async fn probe(&self, path: &Path, token: &CancellationToken) -> Result<Probe> {
        let info = audio_metadata::inspect::inspect(&self.tools, path, token).await?;
        Ok(Probe {
            title: if info.title.is_empty() {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            } else {
                info.title
            },
            artist: info.artist,
            duration: info.duration_seconds,
            format: info.format,
            date: info.date,
            comment: info.comment,
            codec: info.codec,
        })
    }
    pub async fn download(
        &self,
        target: &str,
        directory: &Path,
        token: &CancellationToken,
    ) -> Result<PathBuf> {
        std::fs::create_dir_all(directory)?;
        let pattern = directory.join("%(title).180B.%(ext)s").into_os_string();
        let mut last = None;
        for extractor in [Some("youtube:player_client=mweb,web"), None] {
            let reported = Arc::new(Mutex::new(None));
            let captured = reported.clone();
            let mut command: Vec<std::ffi::OsString> = args(&[
                "--no-warnings",
                "--no-playlist",
                "--no-overwrites",
                "-f",
                "bestaudio/best",
                "--progress",
                "--newline",
                "--progress-template",
                "download:__FETCH_PROGRESS__%(progress)j",
                "--print",
                "after_move:__FETCH_PATH__%(filepath)j",
                "-o",
            ])
            .into_iter()
            .map(Into::into)
            .collect();
            command.push(pattern.clone());
            if let Some(extractor) = extractor {
                command.extend(["--extractor-args", extractor].map(std::ffi::OsString::from));
            }
            command.push(target.into());
            let reporter = self.reporter.clone();
            let result = self
                .tools
                .lines::<_, _, Error>(
                    "yt-dlp",
                    &command,
                    Duration::from_secs(300),
                    token,
                    move |line| {
                        let reporter = reporter.clone();
                        let captured = captured.clone();
                        async move {
                            if let Some(json) = line.strip_prefix("__FETCH_PATH__") {
                                let path: String = serde_json::from_str(json)?;
                                *captured.lock().expect("download result lock") =
                                    Some(PathBuf::from(path));
                            }
                            if let Some(json) = line.strip_prefix("__FETCH_PROGRESS__") {
                                let value: Value = serde_json::from_str(json)?;
                                let total = value["total_bytes"]
                                    .as_f64()
                                    .or_else(|| value["total_bytes_estimate"].as_f64());
                                let percent = total.filter(|v| *v > 0.0).and_then(|n| {
                                    value["downloaded_bytes"]
                                        .as_f64()
                                        .map(|v| (v / n * 100.0).clamp(0.0, 100.0))
                                });
                                let mut event = Event::new("progress", "download");
                                event.percent = percent;
                                event.message = text(&value["status"]);
                                reporter.emit(event, token).await?;
                            }
                            Ok(())
                        }
                    },
                )
                .await;
            match result {
                Ok(()) => {
                    let downloaded = reported
                        .lock()
                        .expect("download result lock")
                        .take()
                        .ok_or_else(|| {
                            Error::Process("yt-dlp did not report its output path".into())
                        })?;
                    let downloaded = if downloaded.is_absolute() {
                        downloaded
                    } else {
                        self.tools.cwd.join(downloaded)
                    };
                    if !downloaded.is_file() {
                        return Err(Error::Process(
                            "yt-dlp reported a missing output file".into(),
                        ));
                    }
                    let root = std::fs::canonicalize(directory)?;
                    if !std::fs::canonicalize(&downloaded)?.starts_with(root) {
                        return Err(Error::Process(
                            "yt-dlp output escaped the download directory".into(),
                        ));
                    }
                    let probe = self.probe(&downloaded, token).await?;
                    let ext = match probe.codec.as_str() {
                        "aac" | "alac" => "m4a",
                        "opus" => "opus",
                        "vorbis" => "ogg",
                        "mp3" => "mp3",
                        "flac" => "flac",
                        s if s.starts_with("pcm_") => "wav",
                        other => {
                            return Err(Error::Invalid(format!(
                                "unsupported native audio codec {other:?}"
                            )));
                        }
                    };
                    let output = directory
                        .join(downloaded.file_name().expect("downloaded filename"))
                        .with_extension(ext);
                    if output == downloaded {
                        return Ok(downloaded);
                    }
                    let mut command = ["-v", "error", "-n", "-i"]
                        .map(std::ffi::OsString::from)
                        .to_vec();
                    command.push(downloaded.as_os_str().to_owned());
                    command.extend(["-map", "0:a:0", "-c:a", "copy"].map(std::ffi::OsString::from));
                    command.push(output.as_os_str().to_owned());
                    self.tools
                        .run("ffmpeg", &command, 1024, Duration::from_secs(60), token)
                        .await?;
                    return Ok(output);
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or(Error::NoCandidate))
    }
    pub async fn verify_file(
        &self,
        path: &Path,
        token: &CancellationToken,
    ) -> Result<Verification> {
        let metadata = self.probe(path, token).await?;
        let mut command: Vec<std::ffi::OsString> = args(&[
            "-v",
            "error",
            "-ss",
            if metadata.duration < 35.0 { "0" } else { "30" },
            "-i",
        ])
        .into_iter()
        .map(Into::into)
        .collect();
        command.push(path.as_os_str().to_owned());
        command.extend(
            args(&[
                "-t", "30", "-map", "0:a:0", "-f", "f32le", "-ac", "1", "-ar", "48000", "pipe:1",
            ])
            .into_iter()
            .map(std::ffi::OsString::from),
        );
        let bytes = self
            .tools
            .run(
                "ffmpeg",
                &command,
                30 * 48000 * 4,
                Duration::from_secs(30),
                token,
            )
            .await?;
        if bytes.len() % 4 != 0 {
            return Err(Error::Invalid("truncated float PCM".into()));
        }
        let samples: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let quality = quality::analyze(&samples, 48000)?;
        let structure = quality::structure(&metadata.title, metadata.duration);
        let status = if quality.bandwidth_rating == "Insufficient signal" {
            "UNKNOWN"
        } else if structure.radio_edit || quality.low_bandwidth_warning {
            "WARNING"
        } else {
            "PASS"
        }
        .into();
        Ok(Verification {
            metadata,
            structure,
            quality,
            status,
        })
    }
    pub async fn verify(&self, target: &str, token: &CancellationToken) -> Result<Verification> {
        if let Some(url) = crate::source_url(target) {
            let tmp = Temporary::new(&self.tools.cwd)?;
            let file = self.download(url.as_str(), tmp.path(), token).await?;
            self.verify_file(&file, token).await
        } else {
            self.verify_file(Path::new(target), token).await
        }
    }
}
