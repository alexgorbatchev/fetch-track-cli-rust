//! Cancellable NDJSON delivery. Event schemas belong to callers.
use std::sync::Arc;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::Mutex,
};
pub use tokio_util::sync::CancellationToken;
mod fd;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Stdout,
    Stderr,
    Unix(std::path::PathBuf),
    Tcp(String),
    Fd(i32),
}
impl Target {
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        let invalid = || Error::Invalid(format!("invalid progress target {raw:?}"));
        match raw {
            "stdout" | "stdout://" => Ok(Self::Stdout),
            "stderr" | "stderr://" => Ok(Self::Stderr),
            "" => Err(invalid()),
            _ if raw.starts_with("fd://") => raw[5..]
                .parse::<i32>()
                .ok()
                .filter(|n| *n >= 0)
                .map(Self::Fd)
                .ok_or_else(invalid),
            _ if raw.starts_with("unix://") => {
                let path = &raw[7..];
                if path.is_empty() {
                    Err(invalid())
                } else {
                    Ok(Self::Unix(path.into()))
                }
            }
            _ if raw.starts_with("tcp://") => {
                let addr = &raw[6..];
                if addr.is_empty() {
                    Err(invalid())
                } else {
                    Ok(Self::Tcp(addr.into()))
                }
            }
            _ if raw.contains("://") => Err(invalid()),
            _ if raw.starts_with('/')
                || raw.starts_with("./")
                || raw.starts_with("../")
                || raw.ends_with(".sock") =>
            {
                Ok(Self::Unix(raw.into()))
            }
            _ if raw.contains(':') => Ok(Self::Tcp(raw.into())),
            _ => Err(invalid()),
        }
    }
}
#[derive(Clone, Default)]
pub struct Reporter {
    writer: Option<Arc<Mutex<Box<dyn AsyncWrite + Send + Unpin>>>>,
}
impl Reporter {
    pub fn writer(writer: impl AsyncWrite + Send + Unpin + 'static) -> Self {
        Self {
            writer: Some(Arc::new(Mutex::new(Box::new(writer)))),
        }
    }
    pub async fn connect(target: &Target, token: &CancellationToken) -> Result<Self> {
        let connect = async {
            Ok(match target {
                Target::Stdout => Self::writer(fd::writer(libc::STDOUT_FILENO)?),
                Target::Stderr => Self::writer(fd::writer(libc::STDERR_FILENO)?),
                Target::Unix(path) => Self::writer(tokio::net::UnixStream::connect(path).await?),
                Target::Tcp(address) => {
                    Self::writer(tokio::net::TcpStream::connect(address).await?)
                }
                Target::Fd(fd) => Self::writer(fd::writer(*fd)?),
            })
        };
        tokio::select! {biased;_=token.cancelled()=>Err(Error::Cancelled),r=tokio::time::timeout(std::time::Duration::from_secs(10),connect)=>r.map_err(|_|Error::Invalid("progress connection timed out".into()))?}
    }
    pub async fn emit<T: serde::Serialize>(
        &self,
        event: T,
        token: &CancellationToken,
    ) -> Result<()> {
        let Some(writer) = &self.writer else {
            return Ok(());
        };
        let mut line = serde_json::to_vec(&event)?;
        line.push(b'\n');
        tokio::select! {biased;_=token.cancelled()=>Err(Error::Cancelled),r=async{let mut writer=writer.lock().await;writer.write_all(&line).await?;writer.flush().await}=>Ok(r?)}
    }
}
