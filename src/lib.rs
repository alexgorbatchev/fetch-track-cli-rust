pub mod candidate;
pub mod cli;
pub mod pipeline;
pub mod progress;
pub mod quality;
pub mod update;

use std::path::Path;
use thiserror::Error;
pub use tokio_util::sync::CancellationToken;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Process(String),
    #[error("{0}")]
    Metadata(String),
    #[error("no matching candidate found")]
    NoCandidate,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    Dependency(#[from] cli_deps::Error),
    #[error("radio edit warning")]
    RadioEdit,
}
pub type Result<T> = std::result::Result<T, Error>;

pub fn source_url(raw: &str) -> Option<reqwest::Url> {
    if let Ok(url) = reqwest::Url::parse(raw)
        && matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
    {
        return Some(url);
    }
    let url = reqwest::Url::parse(&format!("https://{raw}")).ok()?;
    let host = url.host_str()?;
    [
        "youtube.com",
        "youtu.be",
        "soundcloud.com",
        "bandcamp.com",
        "mixcloud.com",
    ]
    .iter()
    .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    .then_some(url)
}

pub fn display(path: &Path) -> String {
    cli_command_kit::display_path(path)
}
pub fn check_cancel(token: &CancellationToken) -> Result<()> {
    if token.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

pub fn split_query(query: &str) -> (&str, &str) {
    query
        .split_once(" - ")
        .map(|(a, t)| (a.trim(), t.trim()))
        .unwrap_or(("", query.trim()))
}

impl From<audio_metadata::Error> for Error {
    fn from(error: audio_metadata::Error) -> Self {
        match error {
            audio_metadata::Error::Invalid(v) => Self::Invalid(v),
            audio_metadata::Error::Process(v) => Self::Process(v),
            audio_metadata::Error::Metadata(v) => Self::Metadata(v),
            audio_metadata::Error::Io(v) => Self::Io(v),
            audio_metadata::Error::Json(v) => Self::Json(v),
            audio_metadata::Error::Http(v) => Self::Http(v),
            audio_metadata::Error::Cancelled => Self::Cancelled,
        }
    }
}

impl From<cli_event_stream::Error> for Error {
    fn from(error: cli_event_stream::Error) -> Self {
        match error {
            cli_event_stream::Error::Invalid(v) => Self::Invalid(v),
            cli_event_stream::Error::Cancelled => Self::Cancelled,
            cli_event_stream::Error::Io(v) => Self::Io(v),
            cli_event_stream::Error::Json(v) => Self::Json(v),
        }
    }
}
