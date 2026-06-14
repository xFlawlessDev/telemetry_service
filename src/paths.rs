use std::{env, path::PathBuf};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub state_file: PathBuf,
    pub log_dir: PathBuf,
    pub log_file: PathBuf,
}

impl AppPaths {
    #[must_use]
    pub fn discover() -> Self {
        let data_dir = data_dir();
        let state_file = data_dir.join("activation_state.json");
        let log_dir = data_dir.join("logs");
        let log_file = log_dir.join("activation.log");

        Self {
            data_dir,
            state_file,
            log_dir,
            log_file,
        }
    }
}

fn data_dir() -> PathBuf {
    env::var_os("PROGRAMDATA")
        .map(PathBuf::from)
        .or_else(|| env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .map(|base| base.join("TelemetryService"))
        .unwrap_or_else(|| PathBuf::from(".").join("TelemetryService"))
}
