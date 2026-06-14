use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::error::{AppResult, io_error};

pub fn init_logging(log_dir: &Path) -> AppResult<WorkerGuard> {
    std::fs::create_dir_all(log_dir).map_err(|source| io_error(log_dir, source))?;
    let file_appender = tracing_appender::rolling::never(log_dir, "activation.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .init();

    Ok(guard)
}
