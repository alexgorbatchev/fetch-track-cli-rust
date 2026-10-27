use crate::{Error, Result};

/// The CLI owns its environment policy; the dependency library owns updates.
pub fn from_values(
    base: Option<String>,
    owner: Option<String>,
    repository: Option<String>,
) -> Result<Option<cli_deps::ReleaseChannel>> {
    match (base, owner, repository) {
        (None, None, None) => Ok(None),
        (Some(base), Some(owner), Some(repository)) => {
            Ok(Some(cli_deps::ReleaseChannel::new(&base, owner, repository)?))
        }
        _ => Err(Error::Invalid("set FETCH_TRACK_RUST_RELEASE_BASE_URL, FETCH_TRACK_RUST_RELEASE_OWNER, and FETCH_TRACK_RUST_RELEASE_REPO together".into())),
    }
}
pub fn from_env() -> Result<cli_deps::Updater> {
    Ok(cli_deps::Updater {
        channel: from_values(
            std::env::var("FETCH_TRACK_RUST_RELEASE_BASE_URL").ok(),
            std::env::var("FETCH_TRACK_RUST_RELEASE_OWNER").ok(),
            std::env::var("FETCH_TRACK_RUST_RELEASE_REPO").ok(),
        )?,
    })
}
