use std::{env, path::Path};

use auto_launch::{AutoLaunch, AutoLaunchBuilder, WindowsEnableMode};
use tokio::process::Command;
use tracing::warn;

use crate::error::{AppError, AppResult};

pub async fn install_autostart(entry_name: &str, executable: &Path) -> AppResult<()> {
    build_auto_launch(entry_name, executable)?.enable()?;
    remove_legacy_task(entry_name).await;
    Ok(())
}

pub async fn disable_autostart(entry_name: &str) -> AppResult<()> {
    let executable = env::current_exe().map_err(|source| AppError::Io {
        path: "current executable".into(),
        source,
    })?;
    build_auto_launch(entry_name, &executable)?.disable()?;
    remove_legacy_task(entry_name).await;
    Ok(())
}

fn build_auto_launch(entry_name: &str, executable: &Path) -> AppResult<AutoLaunch> {
    let quoted_path = format!("\"{}\"", executable.display());
    AutoLaunchBuilder::new()
        .set_app_name(entry_name)
        .set_app_path(&quoted_path)
        .set_windows_enable_mode(WindowsEnableMode::Dynamic)
        .build()
        .map_err(AppError::from)
}

async fn remove_legacy_task(task_name: &str) {
    let output = Command::new("schtasks")
        .args(["/Delete", "/TN", task_name, "/F"])
        .output()
        .await;

    match output {
        Ok(output)
            if output.status.success()
                || task_not_found(&output.stderr)
                || task_not_found(&output.stdout) => {}
        Ok(output) => {
            warn!(
                task = task_name,
                status = output.status.to_string(),
                "legacy scheduled task cleanup failed"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            warn!(task = task_name, %error, "legacy scheduled task cleanup failed");
        }
    }
}

fn task_not_found(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    text.contains("cannot find")
        || text.contains("does not exist")
        || text.contains("tidak dapat menemukan")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_not_found_should_match_english_schtasks_error() {
        assert!(task_not_found(
            b"ERROR: The system cannot find the file specified."
        ));
    }
}
