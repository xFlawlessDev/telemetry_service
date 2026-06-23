#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod autostart;
mod config;
mod error;
mod logging;
mod paths;
mod retry;
mod state;

use std::{env, time::Duration};

use tokio::{fs, time::sleep};

use api::{ActivationClient, ActivationFailure};
use config::AppConfig;
use error::{AppError, AppResult};
use paths::AppPaths;
use state::{load_or_initialize_state, now_utc, save_state_atomic};
use tracing::{error, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliCommand {
    InstallTask,
    RemoveTask,
    ResetState,
}

#[derive(Debug, Clone, Copy, Default)]
struct RuntimeOptions {
    once: bool,
    print_payload: bool,
    command: Option<CliCommand>,
}

#[tokio::main]
async fn main() {
    let config = AppConfig::production();
    let paths = AppPaths::discover();
    let options = parse_options(env::args().skip(1));

    if let Some(command) = options.command {
        if let Err(error) = run_cli_command(command, &config, &paths).await {
            eprintln!("command failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    let _log_guard = match logging::init_logging(&paths.log_dir) {
        Ok(guard) => Some(guard),
        Err(error) => {
            eprintln!("failed to initialize file logging: {error}");
            None
        }
    };

    if let Err(error) = run(config, paths, options).await {
        error!(%error, "activation agent failed");
        std::process::exit(1);
    }
}

async fn run_cli_command(
    command: CliCommand,
    config: &AppConfig,
    paths: &AppPaths,
) -> AppResult<()> {
    match command {
        CliCommand::InstallTask => {
            let executable = env::current_exe().map_err(|source| AppError::Io {
                path: "current executable".into(),
                source,
            })?;
            autostart::install_autostart(config.task_name, &executable).await?;
            println!("installed scheduled task `{}`", config.task_name);
        }
        CliCommand::RemoveTask => {
            autostart::disable_autostart(config.task_name).await?;
            println!("removed scheduled task `{}`", config.task_name);
        }
        CliCommand::ResetState => {
            reset_local_state(paths).await?;
            println!("reset local state at `{}`", paths.data_dir.display());
        }
    }
    Ok(())
}

async fn reset_local_state(paths: &AppPaths) -> AppResult<()> {
    remove_file_if_exists(&paths.state_file).await?;
    remove_dir_if_exists(&paths.log_dir).await
}

async fn remove_file_if_exists(path: &std::path::Path) -> AppResult<()> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AppError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

async fn remove_dir_if_exists(path: &std::path::Path) -> AppResult<()> {
    match fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AppError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

async fn run(config: AppConfig, paths: AppPaths, options: RuntimeOptions) -> AppResult<()> {
    info!(state = %paths.state_file.display(), log = %paths.log_file.display(), data_dir = %paths.data_dir.display(), "activation agent startup");
    let mut state = load_or_initialize_state(&paths.state_file).await?;
    info!(install_id = %state.install_id, activated = state.activated, attempts = state.attempt_count, "state loaded");

    if state.activated {
        cleanup_autostart(&config).await?;
        return Ok(());
    }

    let client = ActivationClient::new(&config)?;

    loop {
        state.record_attempt(now_utc());
        save_state_atomic(&paths.state_file, &state).await?;

        match client.activate(state.install_id).await {
            Ok(success) => {
                state.mark_activated(success.device_id);
                save_state_atomic(&paths.state_file, &state).await?;
                info!(device_id = ?state.activation_id, "registration succeeded");
                cleanup_autostart(&config).await?;
                return Ok(());
            }
            Err(ActivationFailure::Fatal(reason)) => {
                state.last_error = Some(reason.clone());
                save_state_atomic(&paths.state_file, &state).await?;
                return Err(AppError::FatalActivation(reason));
            }
            Err(ActivationFailure::Retryable {
                reason,
                retry_after,
            }) => {
                state.last_error = Some(reason.clone());
                save_state_atomic(&paths.state_file, &state).await?;
                warn!(%reason, "registration retryable failure");
                if options.once || !config.retry_forever {
                    return Ok(());
                }
                let delay = retry_after
                    .unwrap_or_else(|| retry::backoff_delay(&config, state.attempt_count));
                sleep_with_log(delay).await;
            }
        }
    }
}

async fn cleanup_autostart(config: &AppConfig) -> AppResult<()> {
    match autostart::disable_autostart(config.task_name).await {
        Ok(()) => {
            info!(task = config.task_name, "autostart disabled");
            Ok(())
        }
        Err(error) => {
            warn!(%error, task = config.task_name, "autostart cleanup failed");
            Err(error)
        }
    }
}

async fn sleep_with_log(delay: Duration) {
    info!(seconds = delay.as_secs(), "sleeping before retry");
    sleep(delay).await;
}

fn parse_options(args: impl IntoIterator<Item = String>) -> RuntimeOptions {
    let mut options = RuntimeOptions::default();
    for arg in args {
        match arg.as_str() {
            "--once" => options.once = true,
            "--print-payload" => options.print_payload = true,
            "--install-task" => options.command = Some(CliCommand::InstallTask),
            "--remove-task" => options.command = Some(CliCommand::RemoveTask),
            "--reset-state" => options.command = Some(CliCommand::ResetState),
            _ => {}
        }
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_options_should_enable_once_and_print_payload() {
        let options = parse_options(["--once".to_owned(), "--print-payload".to_owned()]);

        assert!(options.once);
        assert!(options.print_payload);
    }

    #[test]
    fn parse_options_should_enable_install_task_command() {
        let options = parse_options(["--install-task".to_owned()]);

        assert_eq!(options.command, Some(CliCommand::InstallTask));
    }

    #[test]
    fn parse_options_should_enable_reset_state_command() {
        let options = parse_options(["--reset-state".to_owned()]);

        assert_eq!(options.command, Some(CliCommand::ResetState));
    }
}
