use std::path::Path;

use tracing::{info, warn};

use crate::error::{AppError, AppResult};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
#[cfg(windows)]
const ERROR_ACCESS_DENIED: i32 = 5;
#[cfg(windows)]
const BUILTIN_USERS_SID: &str = "*S-1-5-32-545";

/// Grant `BUILTIN\Users` the right to delete the installed binary and its
/// folder, without granting write access, so the non-elevated agent can remove
/// itself after successful activation.
pub fn grant_user_delete_permissions(executable: &Path) -> AppResult<()> {
    #[cfg(windows)]
    {
        let folder = executable.parent().ok_or_else(|| {
            AppError::Autostart(format!(
                "cannot grant delete permissions: `{}` has no parent directory",
                executable.display()
            ))
        })?;
        run_icacls(folder, &user_delete_permission())
    }

    #[cfg(not(windows))]
    {
        let _ = executable;
        Ok(())
    }
}

/// Spawn a hidden helper that waits for this process to exit and then deletes
/// the installed binary and its folder. Best-effort: failures are logged only.
pub fn schedule_app_removal(executable: &Path) {
    #[cfg(windows)]
    {
        if let Err(error) = spawn_app_removal_helper(executable) {
            warn!(%error, "failed to schedule application removal");
        }
    }

    #[cfg(not(windows))]
    {
        let _ = executable;
    }
}

#[cfg(windows)]
fn user_delete_permission() -> String {
    format!("{BUILTIN_USERS_SID}:(OI)(CI)(D,DC)")
}

#[cfg(windows)]
fn run_icacls(path: &Path, grant: &str) -> AppResult<()> {
    use std::os::windows::process::CommandExt;

    let output = std::process::Command::new("icacls")
        .arg(path)
        .arg("/grant")
        .arg(grant)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|source| AppError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    if output.status.success() {
        return Ok(());
    }

    Err(AppError::Autostart(format!(
        "icacls /grant failed for `{}` (status {}): {}",
        path.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

#[cfg(windows)]
fn spawn_app_removal_helper(executable: &Path) -> AppResult<()> {
    let Some(folder) = executable.parent() else {
        return Err(AppError::Autostart(format!(
            "cannot schedule app removal: `{}` has no parent directory",
            executable.display()
        )));
    };

    let script_path = std::env::temp_dir().join(format!(
        "telemetry_cleanup_{}.cmd",
        uuid::Uuid::new_v4().simple()
    ));
    let script = app_removal_script(executable, folder);
    std::fs::write(&script_path, script).map_err(|source| AppError::Io {
        path: script_path.clone(),
        source,
    })?;

    spawn_detached(&script_path)?;
    info!(
        executable = %executable.display(),
        "application removal scheduled after exit"
    );
    Ok(())
}

#[cfg(windows)]
fn app_removal_script(executable: &Path, folder: &Path) -> String {
    let exe = executable.display();
    let folder = folder.display();
    format!(
        "@echo off\r\n\
ping -n 3 127.0.0.1 >nul\r\n\
del /f /q \"{exe}\" >nul 2>&1\r\n\
ping -n 3 127.0.0.1 >nul\r\n\
del /f /q \"{exe}\" >nul 2>&1\r\n\
rmdir /s /q \"{folder}\" >nul 2>&1\r\n\
del /f /q \"%~f0\" >nul 2>&1\r\n"
    )
}

#[cfg(windows)]
fn spawn_detached(script_path: &Path) -> AppResult<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let run = |flags: u32| {
        Command::new("cmd.exe")
            .arg("/d")
            .arg("/c")
            .arg(script_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
            .map(|_| ())
    };

    match run(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) => run(CREATE_NO_WINDOW)
            .map_err(|source| AppError::Io {
                path: script_path.to_path_buf(),
                source,
            }),
        Err(source) => Err(AppError::Io {
            path: script_path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::path::Path;

    #[cfg(windows)]
    use super::*;

    #[cfg(windows)]
    #[test]
    fn app_removal_script_should_target_executable_and_folder() {
        let script = app_removal_script(
            Path::new(r"C:\Program Files\TelemetryService\telemetry_service.exe"),
            Path::new(r"C:\Program Files\TelemetryService"),
        );

        assert!(
            script
                .contains(r#"del /f /q "C:\Program Files\TelemetryService\telemetry_service.exe""#)
        );
        assert!(script.contains(r#"rmdir /s /q "C:\Program Files\TelemetryService""#));
        assert!(script.contains(r#"del /f /q "%~f0""#));
    }

    #[cfg(windows)]
    #[test]
    fn user_delete_permission_should_target_builtin_users_without_write() {
        let permission = user_delete_permission();

        assert!(permission.starts_with(BUILTIN_USERS_SID));
        assert!(permission.contains("D,DC"));
        assert!(!permission.contains('M'));
        assert!(!permission.contains('W'));
        assert!(!permission.contains('F'));
    }
}
