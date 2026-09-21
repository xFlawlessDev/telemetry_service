#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod autostart;
mod cleanup;
mod config;
mod error;
mod hardware;
mod location;
mod logging;
mod paths;
mod retry;
mod state;

use std::{env, path::Path, time::Duration};

use tokio::{fs, time::sleep};
use tracing::{error, info, warn};

use api::{ActivationClient, ActivationFailure, DeviceRegistration, payload_debug_string};
use config::{AppConfig, DEBUG};
use error::{AppError, AppResult};
use hardware::collect_hardware_identity;
use location::BlockDecision;
use paths::AppPaths;
use state::{load_existing_or_new_state, now_utc, save_state_atomic};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliCommand {
    InstallStartup,
    RemoveStartup,
    ResetState,
}

#[derive(Debug, Clone, Copy, Default)]
struct RuntimeOptions {
    once: bool,
    print_payload: bool,
    dry_run: bool,
    self_delete_on_success: bool,
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

    if options.dry_run {
        match collect_device_registration(&config).await {
            Ok(device) => println!("dry-run (no POST): {}", payload_debug_string(&device)),
            Err(error) => {
                eprintln!("dry-run failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    let _log_guard = match logging::init_logging(&paths.log_dir) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("failed to initialize file logging: {error}");
            None
        }
    };

    if let Err(error) = run(config, paths, options).await {
        error!(%error, "activation agent failed");
        eprintln!("activation agent failed: {error}");
        std::process::exit(1);
    }
}

async fn run_cli_command(
    command: CliCommand,
    config: &AppConfig,
    paths: &AppPaths,
) -> AppResult<()> {
    match command {
        CliCommand::InstallStartup => {
            let executable = env::current_exe().map_err(|source| AppError::Io {
                path: "current executable".into(),
                source,
            })?;
            autostart::install_autostart(config.task_name, &executable)?;
            cleanup::grant_user_delete_permissions(&executable)?;
            println!("installed startup task `{}`", config.task_name);
        }
        CliCommand::RemoveStartup => {
            autostart::disable_autostart(config.task_name)?;
            println!("removed startup task `{}`", config.task_name);
        }
        CliCommand::ResetState => {
            reset_local_state(paths).await?;
            println!("reset local state at `{}`", paths.data_dir.display());
        }
    }
    Ok(())
}

async fn reset_local_state(paths: &AppPaths) -> AppResult<()> {
    remove_dir_if_exists(&paths.data_dir).await
}

async fn remove_dir_if_exists(path: &Path) -> AppResult<()> {
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
    info!(
        debug = DEBUG,
        state = %paths.state_file.display(),
        log = %paths.log_file.display(),
        data_dir = %paths.data_dir.display(),
        "activation agent startup"
    );

    if DEBUG {
        run_with_state(config, paths, options).await
    } else {
        run_stateless(config, options).await
    }
}

/// Production path: writes no local files at all. The installation is removed
/// after success and logs are disabled, so `--self-delete-on-success` leaves
/// the machine clean. The install id is derived from the hardware serial
/// number, keeping the activation request idempotent across retries.
async fn run_stateless(config: AppConfig, options: RuntimeOptions) -> AppResult<()> {
    let device = collect_device_registration(&config).await?;
    if options.print_payload {
        println!("{}", payload_debug_string(&device));
    }

    let install_id = state::derive_install_id(&device.serial_number);
    let client = ActivationClient::new(&config)?;
    let mut attempt_count: u64 = 0;

    loop {
        if wait_until_outside_block_zone(&config).await {
            return Ok(());
        }

        attempt_count = attempt_count.saturating_add(1);
        match client.activate(install_id, &device).await {
            Ok(_) => {
                info!(%install_id, "registration succeeded");
                cleanup_autostart(&config).await?;
                schedule_app_removal_if_requested(options);
                return Ok(());
            }
            Err(ActivationFailure::Fatal(reason)) => {
                return Err(AppError::FatalActivation(reason));
            }
            Err(ActivationFailure::Retryable {
                retry_after,
                reason,
            }) => {
                warn!(%reason, "registration retryable failure");
                if options.once || !config.retry_forever {
                    return Ok(());
                }
                let delay =
                    retry_after.unwrap_or_else(|| retry::backoff_delay(&config, attempt_count));
                sleep_with_log(delay).await;
            }
        }
    }
}

/// Debug path: durable JSON state with install id and attempt counters.
async fn run_with_state(
    config: AppConfig,
    paths: AppPaths,
    options: RuntimeOptions,
) -> AppResult<()> {
    let mut state = load_existing_or_new_state(&paths.state_file).await?;
    info!(
        install_id = %state.install_id,
        activated = state.activated,
        attempts = state.attempt_count,
        "state loaded"
    );

    if state.activated {
        cleanup_autostart(&config).await?;
        schedule_app_removal_if_requested(options);
        return Ok(());
    }

    let device = collect_device_registration(&config).await?;
    if options.print_payload {
        let payload = payload_debug_string(&device);
        println!("{payload}");
        info!(%payload, "activation payload");
    }

    let client = ActivationClient::new(&config)?;

    loop {
        if wait_until_outside_block_zone(&config).await {
            return Ok(());
        }

        state.record_attempt(now_utc());

        match client.activate(state.install_id, &device).await {
            Ok(_) => {
                state.mark_activated(state.install_id.to_string());
                save_state_atomic(&paths.state_file, &state).await?;
                info!(activation_id = ?state.activation_id, "registration succeeded");
                cleanup_autostart(&config).await?;
                schedule_app_removal_if_requested(options);
                return Ok(());
            }
            Err(ActivationFailure::Fatal(reason)) => {
                return Err(AppError::FatalActivation(reason));
            }
            Err(ActivationFailure::Retryable {
                reason,
                retry_after,
            }) => {
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

async fn wait_until_outside_block_zone(config: &AppConfig) -> bool {
    let Some(zone) = config.block_zone else {
        return false;
    };

    loop {
        let location = location::get_location(config.geolocation_timeout).await;
        match zone.evaluate(&location) {
            BlockDecision::Clear => return false,
            BlockDecision::Blocked => {
                info!(
                    has_coordinates = location.latitude.is_some() && location.longitude.is_some(),
                    radius_meters = zone.radius_meters,
                    "inside block zone; activation POST suppressed"
                );
            }
            BlockDecision::Unknown => {
                warn!(
                    access_status = %location.access_status,
                    "block zone active but coordinates unavailable; activation POST suppressed"
                );
            }
        }
        sleep_with_log(config.block_zone_poll_interval).await;
    }
}

async fn collect_device_registration(config: &AppConfig) -> AppResult<DeviceRegistration> {
    let hardware = collect_hardware_identity()?;
    let serial_number = hardware.serial_number().ok_or_else(|| {
        AppError::FatalActivation("no usable hardware serial number found".to_owned())
    })?;
    let location = location::get_location(config.geolocation_timeout).await;
    if location.latitude.is_none() || location.longitude.is_none() {
        warn!(
            access_status = %location.access_status,
            "geolocation unavailable; coordinates will be sent as null"
        );
    }
    info!(
        has_identifier = hardware.has_identifier(),
        serial_number,
        access_status = %location.access_status,
        has_coordinates = location.latitude.is_some() && location.longitude.is_some(),
        "device identity collected"
    );
    Ok(DeviceRegistration {
        serial_number: serial_number.to_owned(),
        latitude: location.latitude,
        longitude: location.longitude,
        accuracy_meters: location.accuracy_meters,
    })
}

async fn cleanup_autostart(config: &AppConfig) -> AppResult<()> {
    match autostart::disable_autostart(config.task_name) {
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

fn schedule_app_removal_if_requested(options: RuntimeOptions) {
    if !options.self_delete_on_success {
        return;
    }

    match env::current_exe() {
        Ok(executable) => cleanup::schedule_app_removal(&executable),
        Err(error) => {
            warn!(%error, "cannot schedule application removal: current executable unavailable");
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
            "--dry-run" => options.dry_run = true,
            "--self-delete-on-success" => options.self_delete_on_success = true,
            "--install-startup" => options.command = Some(CliCommand::InstallStartup),
            "--remove-startup" => options.command = Some(CliCommand::RemoveStartup),
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
    fn parse_options_should_enable_dry_run() {
        let options = parse_options(["--dry-run".to_owned()]);

        assert!(options.dry_run);
    }

    #[test]
    fn parse_options_should_enable_self_delete_on_success() {
        let options = parse_options(["--self-delete-on-success".to_owned()]);

        assert!(options.self_delete_on_success);
    }

    #[test]
    fn parse_options_should_enable_install_startup_command() {
        let options = parse_options(["--install-startup".to_owned()]);

        assert_eq!(options.command, Some(CliCommand::InstallStartup));
    }

    #[test]
    fn parse_options_should_enable_reset_state_command() {
        let options = parse_options(["--reset-state".to_owned()]);

        assert_eq!(options.command, Some(CliCommand::ResetState));
    }
}
