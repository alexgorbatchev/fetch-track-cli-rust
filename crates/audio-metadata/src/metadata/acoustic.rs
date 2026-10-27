use super::*;
impl Resolver {
    pub async fn acoustid(&self, path: &Path, token: &CancellationToken) -> Result<Metadata> {
        let mut command: Vec<std::ffi::OsString> = args(&["-v", "error", "-i"])
            .into_iter()
            .map(Into::into)
            .collect();
        command.push(path.as_os_str().to_owned());
        command.extend(
            args(&[
                "-map", "0:a:0", "-f", "s16le", "-ac", "1", "-ar", "11025", "pipe:1",
            ])
            .into_iter()
            .map(std::ffi::OsString::from),
        );
        let pcm = self
            .tools
            .run(
                "ffmpeg",
                &command,
                90 * 1024 * 1024,
                Duration::from_secs(45),
                token,
            )
            .await?;
        if pcm.len() % 2 != 0 {
            return Err(Error::Metadata("truncated fingerprint PCM".into()));
        }
        let mut fp = audio_chromaprint::Context::new(audio_chromaprint::Algorithm::Test2);
        fp.start(11025, 1)
            .map_err(|e| Error::Metadata(e.to_string()))?;
        for chunk in pcm.chunks(65536) {
            let samples: Vec<_> = chunk
                .as_chunks::<2>()
                .0
                .iter()
                .map(|v| i16::from_le_bytes(*v))
                .collect();
            fp.feed(&samples)
                .map_err(|e| Error::Metadata(e.to_string()))?;
        }
        fp.finish().map_err(|e| Error::Metadata(e.to_string()))?;
        let fingerprint = fp
            .fingerprint()
            .map_err(|e| Error::Metadata(e.to_string()))?;
        let duration = (pcm.len() / 2 / 11025).to_string();
        let cache_key = format!("{duration}:{fingerprint}");
        crate::check_cancel(token)?;
        if let Some(cached) = self.cache.get("acoustid", &cache_key)? {
            return Ok(cached);
        }
        let data = self
            .request(
                self.http.post(self.endpoints.acoustid.clone()).form(&[
                    ("client", "HSqh_oejCAM".to_owned()),
                    ("duration", duration),
                    ("fingerprint", fingerprint),
                    ("meta", "recordings releases releasegroups".into()),
                ]),
                token,
            )
            .await?;
        if data["status"] != "ok" {
            return Err(Error::Metadata("AcoustID lookup rejected".into()));
        }
        let best = values(&data["results"])
            .iter()
            .filter(|v| {
                v["score"].as_f64().unwrap_or(0.0) >= 0.7
                    && values(&v["recordings"])
                        .iter()
                        .any(|r| !text(&r["title"]).is_empty())
            })
            .max_by(|a, b| {
                a["score"]
                    .as_f64()
                    .partial_cmp(&b["score"].as_f64())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .ok_or_else(|| Error::Metadata("no confident AcoustID recording".into()))?;
        let rec = values(&best["recordings"])
            .iter()
            .find(|r| !text(&r["title"]).is_empty())
            .expect("filtered recording");
        let release = &rec["releases"][0];
        let group = &rec["releasegroups"][0];
        let (kind, id) = if !text(&release["id"]).is_empty() {
            ("release", text(&release["id"]))
        } else {
            ("release-group", text(&group["id"]))
        };
        let cover_url = self.optional_cover(kind, &id, token).await?;
        let d = &release["date"];
        let year = d["year"].as_u64().unwrap_or(0);
        let month = d["month"].as_u64().unwrap_or(0);
        let day = d["day"].as_u64().unwrap_or(0);
        let release_date = if year == 0 {
            String::new()
        } else if month == 0 {
            format!("{year:04}")
        } else if day == 0 {
            format!("{year:04}-{month:02}")
        } else {
            format!("{year:04}-{month:02}-{day:02}")
        };
        let result = Metadata {
            title: text(&rec["title"]),
            artist: values(&rec["artists"])
                .iter()
                .map(|a| text(&a["name"]))
                .collect::<Vec<_>>()
                .join(", "),
            album: if text(&release["title"]).is_empty() {
                text(&group["title"])
            } else {
                text(&release["title"])
            },
            release_date: date(&release_date),
            cover_url,
            provider: "AcoustID / MusicBrainz".into(),
            ..Default::default()
        };
        crate::check_cancel(token)?;
        self.cache.put(
            "acoustid",
            &cache_key,
            &result,
            Duration::from_secs(30 * 86400),
        )?;
        Ok(result)
    }
    pub async fn shazam(&self, path: &Path, token: &CancellationToken) -> Result<Metadata> {
        let config = shazam_client::Config {
            timeout: Duration::from_secs(15),
            decoder: shazam_client::audio::Decoder {
                ffmpeg: self.tools.program("ffmpeg")?,
            },
            endpoints: self.endpoints.shazam.clone(),
            ..Default::default()
        };
        let client =
            shazam_client::Client::new(config).map_err(|e| Error::Metadata(e.to_string()))?;
        let recognition = client.recognize_file(path, token).await.map_err(|e| {
            if matches!(e, shazam_client::Error::Cancelled) {
                Error::Cancelled
            } else {
                Error::Metadata(e.to_string())
            }
        })?;
        let track = recognition
            .track
            .filter(|t| !t.title.is_empty() && !t.subtitle.is_empty())
            .ok_or_else(|| Error::Metadata("no Shazam recording".into()))?;
        let mut result = Metadata {
            title: track.title,
            artist: track.subtitle,
            cover_url: shazam_artwork(&track.photo_url, &track.extra),
            provider: "Shazam".into(),
            genre: text(&track.extra.get("genres").unwrap_or(&Value::Null)["primary"]),
            ..Default::default()
        };
        for section in values(track.extra.get("sections").unwrap_or(&Value::Null)) {
            for field in values(&section["metadata"]) {
                match text(&field["title"]).to_lowercase().as_str() {
                    "album" => result.album = text(&field["text"]),
                    "released" => result.release_date = date(&text(&field["text"])),
                    _ => {}
                }
            }
        }
        Ok(result)
    }
}

fn shazam_artwork(photo: &str, extra: &serde_json::Map<String, Value>) -> String {
    let field = |key: &str| extra.get(key).unwrap_or(&Value::Null);
    let images = field("images");
    let share = field("share");
    let hub = field("hub");
    let candidates = [
        photo.to_owned(),
        text(&images["coverarthq"]),
        text(&images["coverart"]),
        text(&images["background"]),
        text(&share["image"]),
        text(&share["avatar"]),
        text(&hub["image"]),
    ];
    candidates
        .into_iter()
        .chain(
            values(&hub["options"])
                .iter()
                .map(|option| text(&option["image"])),
        )
        .chain(values(field("sections")).iter().flat_map(|section| {
            std::iter::once(text(&section["avatar"])).chain(
                values(&section["metapages"])
                    .iter()
                    .map(|page| text(&page["image"])),
            )
        }))
        .find(|url| !url.is_empty())
        .unwrap_or_default()
        .replacen("400x400cc.jpg", "1400x1400cc.jpg", 1)
        .replacen("800x800cc.jpg", "1400x1400cc.jpg", 1)
}
