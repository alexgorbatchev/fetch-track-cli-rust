use crate::{Error, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct Cache {
    root: PathBuf,
    enabled: bool,
}
#[derive(Serialize, Deserialize)]
struct Entry<T> {
    expires: u64,
    data: T,
}
impl Cache {
    pub fn new(root: PathBuf, enabled: bool) -> Self {
        Self { root, enabled }
    }
    fn path(&self, namespace: &str, key: &str) -> PathBuf {
        self.root
            .join(namespace)
            .join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
    }
    pub fn get<T: DeserializeOwned>(&self, namespace: &str, key: &str) -> Result<Option<T>> {
        if !self.enabled {
            return Ok(None);
        }
        let path = self.path(namespace, key);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let entry: Entry<T> = match serde_json::from_slice(&bytes) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };
        if entry.expires <= epoch()? {
            return Ok(None);
        }
        Ok(Some(entry.data))
    }
    pub fn put<T: Serialize>(
        &self,
        namespace: &str,
        key: &str,
        data: &T,
        ttl: Duration,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let path = self.path(namespace, key);
        let parent = path.parent().expect("cache path parent");
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(
            &mut file,
            &Entry {
                expires: epoch()?.saturating_add(ttl.as_secs()),
                data,
            },
        )?;
        file.flush()?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
    pub fn file_key(path: &Path) -> Result<String> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut digest = Sha256::new();
        let mut bytes = [0; 65536];
        loop {
            let n = file.read(&mut bytes)?;
            if n == 0 {
                break;
            }
            digest.update(&bytes[..n]);
        }
        Ok(format!("{:x}", digest.finalize()))
    }
    /// Read bounded binary data. Corrupt, oversized and expired entries are misses.
    pub fn get_bytes(&self, namespace: &str, key: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        if !self.enabled {
            return Ok(None);
        }
        let path = self.path(namespace, key).with_extension("bin");
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(9) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() < 8 || bytes.len() - 8 > limit {
            return Ok(None);
        }
        let expires = u64::from_le_bytes(bytes[..8].try_into().expect("eight-byte header"));
        if expires <= epoch()? {
            return Ok(None);
        }
        Ok(Some(bytes.split_off(8)))
    }
    pub fn put_bytes(&self, namespace: &str, key: &str, bytes: &[u8], ttl: Duration) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let path = self.path(namespace, key).with_extension("bin");
        let parent = path.parent().expect("cache parent");
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&epoch()?.saturating_add(ttl.as_secs()).to_le_bytes())?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}
fn epoch() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| Error::Invalid(e.to_string()))
}
