use std::{path::Path, process::ExitStatus};

use tokio::process::Command;

use crate::error::{AppError, AppResult};

pub async fn install_autostart(task_name: &str, executable: &Path) -> AppResult<()> {
    let task_run = task_run_argument(executable);
    let output = Command::new("schtasks")
        .args([
            "/Create", "/TN", task_name, "/SC", "ONLOGON", "/RL", "LIMITED", "/TR", &task_run, "/F",
        ])
        .output()
        .await;

    let output = match output {
        Ok(output) => output,
        Err(source) => {
            return Err(AppError::Io {
                path: "schtasks".into(),
                source,
            });
        }
    };

    if output.status.success() {
        return Ok(());
    }

    Err(process_failure(output.status, output.stderr))
}

pub async fn disable_autostart(task_name: &str) -> AppResult<()> {
    let output = Command::new("schtasks")
        .args(["/Delete", "/TN", task_name, "/F"])
        .output()
        .await;

    let output = match output {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(AppError::Io {
                path: "schtasks".into(),
                source,
            });
        }
    };

    if output.status.success() || task_not_found(&output.stderr) || task_not_found(&output.stdout) {
        return Ok(());
    }

    Err(process_failure(output.status, output.stderr))
}

fn process_failure(status: ExitStatus, stderr: Vec<u8>) -> AppError {
    AppError::ProcessFailure {
        program: "schtasks",
        status: status.to_string(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    }
}

fn task_run_argument(executable: &Path) -> String {
    format!("\"{}\"", executable.display())
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

    #[test]
    fn task_run_argument_should_quote_executable_path() {
        let task_run = task_run_argument(Path::new(
            r"C:\Program Files\TelemetryService\telemetry.exe",
        ));

        assert_eq!(
            task_run,
            r#""C:\Program Files\TelemetryService\telemetry.exe""#
        );
    }
}
