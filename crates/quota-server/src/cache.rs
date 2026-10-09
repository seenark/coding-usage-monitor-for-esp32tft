use quota_core::{DataSource, MAX_SNAPSHOT_BYTES, Provider, Snapshot, decode_snapshot};
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::io::AsyncWriteExt;

#[derive(Clone)]
pub struct Cache {
    directory: PathBuf,
}
impl Cache {
    pub async fn new(directory: impl Into<PathBuf>) -> io::Result<Self> {
        let directory = directory.into();
        tokio::fs::create_dir_all(&directory).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).await?;
        }
        Ok(Self { directory })
    }
    pub async fn load(
        &self,
        provider: Provider,
        source: DataSource,
    ) -> io::Result<Option<Snapshot>> {
        let path = self.directory.join(format!("{}.json", provider.as_str()));
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if metadata.len() > MAX_SNAPSHOT_BYTES as u64 {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        let bytes = tokio::fs::read(path).await?;
        let snapshot = decode_snapshot(provider.topic(), &bytes)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        Ok((snapshot.source == source && snapshot.fetched_at.is_some()).then_some(snapshot))
    }
    pub async fn save(&self, snapshot: &Snapshot, encoded: &[u8]) -> io::Result<()> {
        snapshot
            .validate()
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        if encoded.len() > MAX_SNAPSHOT_BYTES {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        write_atomic(
            &self.directory,
            &format!("{}.json", snapshot.provider.as_str()),
            encoded,
        )
        .await
    }
}
/// Temp file + fsync + rename, mode 0600. Shared by every JSON file in the cache directory.
pub async fn write_atomic(directory: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let temporary = directory.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        rand::random::<u64>()
    ));
    let result = write_and_rename(&temporary, &directory.join(name), bytes).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}
async fn write_and_rename(temporary: &Path, destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(temporary).await?;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    drop(file);
    tokio::fs::rename(temporary, destination).await?;
    if let Some(parent) = destination.parent() {
        tokio::fs::File::open(parent).await?.sync_all().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quota_core::{ProviderState, Window};
    #[tokio::test]
    async fn restart_preserves_success_and_rejects_mock_as_live() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::new(directory.path()).await.unwrap();
        let mut snapshot = Snapshot::empty(Provider::Codex, DataSource::Mock, 1000);
        snapshot.state = ProviderState::Ok;
        snapshot.fetched_at = Some(1000);
        snapshot.windows.push(
            Window::new(
                "codex",
                "primary",
                "5-hour",
                Some(300),
                Some(62.0),
                Some(2000),
            )
            .unwrap(),
        );
        cache
            .save(&snapshot, &serde_json::to_vec(&snapshot).unwrap())
            .await
            .unwrap();
        let reopened = Cache::new(directory.path()).await.unwrap();
        let restored = reopened
            .load(Provider::Codex, DataSource::Mock)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.fetched_at, Some(1000));
        assert_eq!(restored.windows[0].remaining_percent, Some(38.0));
        assert!(
            reopened
                .load(Provider::Codex, DataSource::Provider)
                .await
                .unwrap()
                .is_none()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(directory.path().join("codex.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
