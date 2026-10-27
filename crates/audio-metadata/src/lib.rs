//! Audio recognition, bounded subprocesses and atomic native tag editing.
pub mod cache;
pub mod identity;
pub mod inspect;
pub mod metadata;
pub mod process;
pub mod tag;

pub use tokio_util::sync::CancellationToken;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Process(String),
    #[error("{0}")]
    Metadata(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

pub fn check_cancel(token: &CancellationToken) -> Result<()> {
    if token.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
