use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::{fs, io::AsyncWriteExt};
use uuid::Uuid;

use crate::error::{AppError, AppResult, io_error};

/// Namespace used to derive a deterministic, stable install id from the
/// hardware serial number when durable state is disabled. This keeps the
/// activation request idempotent across retries and logons without writing
/// any local file.
const INSTALL_ID_NAMESPACE: Uuid = Uuid::from_u128(0x6f9a1c4e_2b7d_4f0a_9c31_5d8e2a6b7c10);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationState {
    pub install_id: Uuid,
    pub activated: bool,
    pub activation_id: Option<String>,
    pub attempt_count: u64,
    pub first_seen_utc: String,
    pub last_attempt_utc: Option<String>,
    pub last_error: Option<String>,
}

impl ActivationState {
    #[must_use]
    pub fn new(now_utc: String) -> Self {
        Self {
            install_id: Uuid::new_v4(),
            activated: false,
            activation_id: None,
            attempt_count: 0,
            first_seen_utc: now_utc,
            last_attempt_utc: None,
            last_error: None,
        }
    }

    pub fn record_attempt(&mut self, now_utc: String) {
        self.attempt_count = self.attempt_count.saturating_add(1);
        self.last_attempt_utc = Some(now_utc);
    }

    pub fn mark_activated(&mut self, activation_id: String) {
        self.activated = true;
        self.activation_id = Some(activation_id);
        self.last_error = None;
    }
}

/// Derive a stable install id from the hardware serial number. The same device
/// always produces the same id, so retries reuse one server-side idempotency
/// key without persisting anything locally.
#[must_use]
pub fn derive_install_id(serial_number: &str) -> Uuid {
    Uuid::new_v5(&INSTALL_ID_NAMESPACE, serial_number.as_bytes())
}

pub async fn load_existing_or_new_state(path: &Path) -> AppResult<ActivationState> {
    let result = match fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| AppError::StateJson {
            path: path.to_path_buf(),
            source,
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ActivationState::new(now_utc()))
        }
        Err(source) => Err(io_error(path, source)),
    };

    match result {
        Ok(state) => Ok(state),
        Err(AppError::StateJson { .. }) => {
            quarantine_corrupt_state(path).await?;
            Ok(ActivationState::new(now_utc()))
        }
        Err(error) => Err(error),
    }
}

pub async fn save_state_atomic(path: &Path, state: &ActivationState) -> AppResult<()> {
    create_parent_dir(path).await?;

    let tmp_path = tmp_path_for(path);
    let bytes = serde_json::to_vec_pretty(state)?;
    let mut file = fs::File::create(&tmp_path)
        .await
        .map_err(|source| io_error(&tmp_path, source))?;
    file.write_all(&bytes)
        .await
        .map_err(|source| io_error(&tmp_path, source))?;
    file.flush()
        .await
        .map_err(|source| io_error(&tmp_path, source))?;
    file.sync_all()
        .await
        .map_err(|source| io_error(&tmp_path, source))?;
    drop(file);
    fs::rename(&tmp_path, path)
        .await
        .map_err(|source| io_error(path, source))?;
    Ok(())
}

#[must_use]
pub fn now_utc() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

async fn create_parent_dir(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|source| io_error(parent, source))?;
    }
    Ok(())
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let mut tmp_path = path.to_path_buf();
    tmp_path.set_extension("json.tmp");
    tmp_path
}

async fn quarantine_corrupt_state(path: &Path) -> AppResult<()> {
    let timestamp = now_utc().replace([':', '.'], "-");
    let corrupt_path = path.with_extension(format!("json.corrupt.{timestamp}"));
    fs::rename(path, &corrupt_path)
        .await
        .map_err(|source| io_error(&corrupt_path, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_existing_or_new_state_should_not_create_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activation_state.json");

        let state = load_existing_or_new_state(&path).await.unwrap();

        assert!(!state.activated);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn load_existing_or_new_state_should_preserve_install_id_when_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activation_state.json");
        let first = ActivationState::new(now_utc());
        save_state_atomic(&path, &first).await.unwrap();

        let second = load_existing_or_new_state(&path).await.unwrap();

        assert_eq!(first.install_id, second.install_id);
    }

    #[tokio::test]
    async fn load_existing_or_new_state_should_quarantine_corrupt_json_without_creating_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activation_state.json");
        tokio::fs::write(&path, b"not json").await.unwrap();

        let state = load_existing_or_new_state(&path).await.unwrap();

        assert!(!state.activated);
        assert!(!path.exists());
        assert!(dir.path().read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("corrupt")
        }));
    }

    #[test]
    fn derive_install_id_should_be_stable_for_same_serial() {
        assert_eq!(
            derive_install_id("0223290070363009024"),
            derive_install_id("0223290070363009024")
        );
    }

    #[test]
    fn derive_install_id_should_differ_for_different_serials() {
        assert_ne!(
            derive_install_id("0223290070363009024"),
            derive_install_id("0223290070363009025")
        );
    }
}
