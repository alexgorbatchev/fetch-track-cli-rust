mod acoustic;
use crate::{
    CancellationToken, Error, Result,
    cache::Cache,
    identity::normalize,
    process::{Tools, args},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub release_date: String,
    pub cover_url: String,
    pub provider: String,
    pub audio_source_url: String,
    pub fetched_at: DateTime<Utc>,
}
impl Metadata {
    pub fn release_year(&self) -> String {
        self.release_date.get(..4).unwrap_or("").to_owned()
    }
}
#[derive(Clone)]
pub struct Endpoints {
    pub itunes: reqwest::Url,
    pub musicbrainz: reqwest::Url,
    pub covers: reqwest::Url,
    pub acoustid: reqwest::Url,
    pub shazam: shazam_client::Endpoints,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            itunes: "https://itunes.apple.com/search".parse().unwrap(),
            musicbrainz: "https://musicbrainz.org/ws/2/recording/".parse().unwrap(),
            covers: "https://coverartarchive.org/".parse().unwrap(),
            acoustid: "https://api.acoustid.org/v2/lookup".parse().unwrap(),
            shazam: Default::default(),
        }
    }
}
pub struct Resolver {
    pub http: reqwest::Client,
    pub endpoints: Endpoints,
    pub cache: Cache,
    pub tools: Tools,
    mb_last: Arc<Mutex<Option<tokio::time::Instant>>>,
}
fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}
fn values(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn date(raw: &str) -> String {
    let raw = raw.get(..10).unwrap_or(raw);
    if chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").is_ok() {
        return raw.into();
    }
    if raw.len() == 7 && chrono::NaiveDate::parse_from_str(&format!("{raw}-01"), "%Y-%m-%d").is_ok()
    {
        return raw.into();
    }
    if raw.len() == 4 && raw.bytes().all(|b| b.is_ascii_digit()) {
        return raw.into();
    }
    String::new()
}
fn identity(actual: &str, expected: &str) -> bool {
    let a = normalize(actual);
    let e = normalize(expected);
    !a.is_empty() && !e.is_empty() && (a == e || a.contains(&e) || e.contains(&a))
}
impl Resolver {
    pub fn new(tools: Tools, cache: Cache, endpoints: Endpoints, user_agent: &str) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .user_agent(user_agent)
                .build()?,
            endpoints,
            cache,
            tools,
            mb_last: Arc::new(Mutex::new(None)),
        })
    }
    async fn request(
        &self,
        request: reqwest::RequestBuilder,
        token: &CancellationToken,
    ) -> Result<Value> {
        let work = async {
            let response = request.send().await?.error_for_status()?;
            use futures::StreamExt;
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                    return Err(Error::Metadata("metadata response too large".into()));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(serde_json::from_slice(&bytes)?)
        };
        tokio::select! {biased;_=token.cancelled()=>Err(Error::Cancelled),r=work=>r}
    }
    pub async fn artwork(&self, url: &str, token: &CancellationToken) -> Result<Vec<u8>> {
        crate::check_cancel(token)?;
        use futures::StreamExt;
        let url: reqwest::Url = url
            .parse()
            .map_err(|e| Error::Invalid(format!("artwork URL: {e}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Error::Invalid("artwork URL must use HTTP(S)".into()));
        }
        if let Some(bytes) = self
            .cache
            .get_bytes("artwork", url.as_str(), 20 * 1024 * 1024)?
        {
            return Ok(bytes);
        }
        let work = async {
            let mut stream = self
                .http
                .get(url.clone())
                .send()
                .await?
                .error_for_status()?
                .bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if bytes.len() + chunk.len() > 20 * 1024 * 1024 {
                    return Err(Error::Metadata("artwork exceeds 20 MiB".into()));
                }
                bytes.extend_from_slice(&chunk);
            }
            if bytes.is_empty() {
                return Err(Error::Metadata("empty artwork payload".into()));
            }
            self.cache.put_bytes(
                "artwork",
                url.as_str(),
                &bytes,
                Duration::from_secs(30 * 86400),
            )?;
            Ok(bytes)
        };
        tokio::select! {biased;_=token.cancelled()=>Err(Error::Cancelled),r=work=>r}
    }
    pub async fn itunes(
        &self,
        query: &str,
        artist: &str,
        title: &str,
        token: &CancellationToken,
    ) -> Result<Metadata> {
        let data = self
            .request(
                self.http.get(self.endpoints.itunes.clone()).query(&[
                    ("term", query),
                    ("entity", "song"),
                    ("limit", "10"),
                ]),
                token,
            )
            .await?;
        let track = values(&data["results"])
            .iter()
            .find(|v| {
                identity(&text(&v["trackName"]), title)
                    && (artist.is_empty()
                        || normalize(&text(&v["artistName"])) == normalize(artist))
            })
            .ok_or_else(|| Error::Metadata("no matching iTunes recording".into()))?;
        Ok(Metadata {
            title: text(&track["trackName"]),
            artist: text(&track["artistName"]),
            album: text(&track["collectionName"]),
            genre: text(&track["primaryGenreName"]),
            release_date: date(&text(&track["releaseDate"])),
            cover_url: text(&track["artworkUrl100"]).replace("100x100bb", "1400x1400bb"),
            provider: "iTunes API".into(),
            ..Default::default()
        })
    }
    async fn cover(&self, kind: &str, id: &str, token: &CancellationToken) -> Result<String> {
        if id.is_empty() {
            return Ok(String::new());
        }
        let url = self
            .endpoints
            .covers
            .join(&format!("{kind}/{id}"))
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let data = self.request(self.http.get(url), token).await?;
        let image = values(&data["images"])
            .iter()
            .find(|v| v["front"] == true && v["approved"] != false)
            .or_else(|| {
                values(&data["images"])
                    .iter()
                    .find(|v| v["approved"] != false)
            });
        Ok(image.map(|v| text(&v["image"])).unwrap_or_default())
    }
    async fn optional_cover(
        &self,
        kind: &str,
        id: &str,
        token: &CancellationToken,
    ) -> Result<String> {
        match self.cover(kind, id, token).await {
            Ok(v) => Ok(v),
            Err(Error::Cancelled) => Err(Error::Cancelled),
            Err(_) => Ok(String::new()),
        }
    }
    pub async fn musicbrainz(
        &self,
        query: &str,
        artist: &str,
        title: &str,
        token: &CancellationToken,
    ) -> Result<Metadata> {
        {
            let mut last = self.mb_last.lock().await;
            if let Some(t) = *last {
                tokio::select! {biased;_=token.cancelled()=>return Err(Error::Cancelled),_=tokio::time::sleep_until(t+Duration::from_secs(1))=>{}}
            }
            *last = Some(tokio::time::Instant::now());
        }
        let data = self
            .request(
                self.http.get(self.endpoints.musicbrainz.clone()).query(&[
                    ("query", query),
                    ("fmt", "json"),
                    ("limit", "10"),
                ]),
                token,
            )
            .await?;
        let rec = values(&data["recordings"])
            .iter()
            .find(|v| {
                identity(&text(&v["title"]), title)
                    && (artist.is_empty()
                        || values(&v["artist-credit"])
                            .iter()
                            .any(|a| normalize(&text(&a["name"])) == normalize(artist)))
            })
            .ok_or_else(|| Error::Metadata("no matching MusicBrainz recording".into()))?;
        let release = &rec["releases"][0];
        let cover_url = self
            .optional_cover("release", &text(&release["id"]), token)
            .await?;
        Ok(Metadata {
            title: text(&rec["title"]),
            artist: values(&rec["artist-credit"])
                .iter()
                .map(|v| format!("{}{}", text(&v["name"]), text(&v["joinphrase"])))
                .collect(),
            album: text(&release["title"]),
            release_date: date(&text(&release["date"])),
            cover_url,
            provider: "MusicBrainz API".into(),
            ..Default::default()
        })
    }
    pub async fn resolve(
        &self,
        path: &Path,
        query: &str,
        artist: &str,
        title: &str,
        token: &CancellationToken,
    ) -> Result<Metadata> {
        crate::check_cancel(token)?;
        let key = format!("{}:{query}:{artist}:{title}", Cache::file_key(path)?);
        if let Some(value) = self.cache.get("metadata", &key)? {
            return Ok(value);
        }
        let mut acoustic = None;
        match self.shazam(path, token).await {
            Ok(v) => acoustic = Some(v),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => {}
        }
        if acoustic.is_none() {
            match self.acoustid(path, token).await {
                Ok(v) => acoustic = Some(v),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => {}
            }
        }
        let (search, expected_artist, expected_title) = if let Some(ref v) = acoustic {
            (
                format!("{} {}", v.artist, v.title),
                v.artist.as_str(),
                v.title.as_str(),
            )
        } else {
            (query.replace('_', " "), artist, title)
        };
        let mut enriched = None;
        if acoustic.as_ref().is_none_or(|v| v.cover_url.is_empty()) {
            match self
                .itunes(&search, expected_artist, expected_title, token)
                .await
            {
                Ok(v) => enriched = Some(v),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => {}
            }
            if enriched.as_ref().is_none_or(|v| v.cover_url.is_empty()) {
                match self
                    .musicbrainz(&search, expected_artist, expected_title, token)
                    .await
                {
                    Ok(v) => {
                        if let Some(existing) = enriched.as_mut() {
                            existing.cover_url = v.cover_url;
                        } else {
                            enriched = Some(v);
                        }
                    }
                    Err(Error::Cancelled) => return Err(Error::Cancelled),
                    Err(_) => {}
                }
            }
        }
        let result = match (acoustic, enriched) {
            (Some(mut a), Some(b)) => {
                if a.cover_url.is_empty() {
                    a.cover_url = b.cover_url;
                }
                if a.album.is_empty() {
                    a.album = b.album;
                }
                if a.release_date.is_empty() {
                    a.release_date = b.release_date;
                }
                a
            }
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => Metadata {
                title: title.into(),
                artist: artist.into(),
                provider: "Raw fallback".into(),
                ..Default::default()
            },
        };
        self.cache
            .put("metadata", &key, &result, Duration::from_secs(7 * 86400))?;
        Ok(result)
    }
}
