use super::*;
impl Pipeline {
    pub async fn probe_url(&self, url: &str, token: &CancellationToken) -> Result<Probe> {
        if let Some(probe) = self.cache.get("url_meta", url)? {
            return Ok(probe);
        }
        let mut command = args(&[
            "--dump-single-json",
            "--no-playlist",
            "--no-warnings",
            "--quiet",
        ]);
        command.push(url.into());
        let bytes = self
            .tools
            .run(
                "yt-dlp",
                &command,
                4 * 1024 * 1024,
                Duration::from_secs(30),
                token,
            )
            .await?;
        let data: Value = serde_json::from_slice(&bytes)?;
        let probe = Probe {
            title: text(&data["title"]),
            artist: if text(&data["artist"]).is_empty() {
                text(&data["uploader"])
            } else {
                text(&data["artist"])
            },
            duration: data["duration"].as_f64().unwrap_or(0.0),
            format: text(&data["ext"]),
            ..Default::default()
        };
        self.cache
            .put("url_meta", url, &probe, Duration::from_secs(86400))?;
        Ok(probe)
    }
    async fn search_one(
        &self,
        source: &str,
        query: &str,
        title: &str,
        token: &CancellationToken,
    ) -> Result<Vec<Candidate>> {
        let prefix = match source {
            "youtube" => "ytsearch8",
            "soundcloud" => "scsearch8",
            _ => {
                return Err(Error::Invalid(format!(
                    "unsupported search source {source:?}; use a direct Bandcamp URL"
                )));
            }
        };
        let key = format!("{source}:{query}");
        if let Some(candidates) = self.cache.get("searches", &key)? {
            return Ok(candidates);
        }
        let mut command = args(&["--flat-playlist", "--dump-json", "--no-warnings", "--quiet"]);
        command.push(format!("{prefix}:{query}"));
        let bytes = self
            .tools
            .run(
                "yt-dlp",
                &command,
                8 * 1024 * 1024,
                Duration::from_secs(8),
                token,
            )
            .await?;
        let mut candidates = Vec::new();
        for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
            let data: Value = serde_json::from_slice(line)?;
            let duration = data["duration"].as_f64().unwrap_or(0.0);
            let name = text(&data["title"]);
            if !(60.0..=900.0).contains(&duration)
                || !candidate::matches(&candidate::clean_title(title), &name)
            {
                continue;
            }
            let id = text(&data["id"]);
            let mut url = text(&data["webpage_url"]);
            if url.is_empty() {
                url = text(&data["url"]);
            }
            if url.is_empty() && source == "youtube" && !id.is_empty() {
                url = format!("https://www.youtube.com/watch?v={id}");
            }
            if !url.is_empty() {
                candidates.push(Candidate {
                    id,
                    title: name,
                    duration,
                    source: source.into(),
                    url,
                    score: 0,
                });
            }
        }
        self.cache
            .put("searches", &key, &candidates, Duration::from_secs(43200))?;
        Ok(candidates)
    }
    pub async fn search(
        &self,
        sources: &[String],
        query: &str,
        artist: &str,
        title: &str,
        token: &CancellationToken,
    ) -> Result<Vec<Candidate>> {
        let query = if !artist.is_empty() && !title.is_empty() {
            format!("{artist} {}", candidate::clean_title(title))
        } else {
            query.into()
        };
        let mut jobs = FuturesUnordered::new();
        for source in sources {
            jobs.push(self.search_one(source, &query, title, token));
        }
        let mut candidates = Vec::new();
        let mut failures = Vec::new();
        while let Some(result) = jobs.next().await {
            match result {
                Ok(found) => {
                    for c in &found {
                        let mut e = Event::new("candidate_found", "search");
                        e.candidate = Some(c.into());
                        self.reporter.emit(e, token).await?;
                    }
                    candidates.extend(found);
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => failures.push(e.to_string()),
            }
        }
        if candidates.is_empty() {
            if failures.is_empty() {
                Err(Error::NoCandidate)
            } else {
                Err(Error::Process(failures.join("; ")))
            }
        } else {
            Ok(candidate::rank(candidates, artist, title))
        }
    }
}
