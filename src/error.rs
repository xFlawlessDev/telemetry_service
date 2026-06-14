use std::{io, path::PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("I/O error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("state JSON error at {path}: {source}")]
    StateJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[cfg(windows)]
    #[error("WMI error: {0}")]
    Wmi(#[from] wmi::WMIError),
    #[cfg(windows)]
    #[error("Windows API error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("HTTP client error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("process `{program}` failed with status {status}: {stderr}")]
    ProcessFailure {
        program: &'static str,
        status: String,
        stderr: String,
    },
    #[error("activation failed permanently: {0}")]
    FatalActivation(String),
}

pub type AppResult<T> = Result<T, AppError>;

pub fn io_error(path: impl Into<PathBuf>, source: io::Error) -> AppError {
    AppError::Io {
        path: path.into(),
        source,
    }
}
