use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tracing::{Event, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter, Layer, fmt, layer::SubscriberExt, registry::LookupSpan, util::SubscriberInitExt,
};

use crate::config::DEBUG;
use crate::error::{AppResult, io_error};

/// Keeps the activation log payload bounded so a long retry loop cannot grow
/// the multipart body without limit. Only the most recent records are sent.
pub const MAX_LOG_RECORDS: usize = 200;

/// One structured tracing event captured for the server-side `logs` field.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogRecord {
    pub timestamp_utc: String,
    pub level: String,
    pub target: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<String>,
}

/// Shared in-memory ring buffer of structured logs. Production mode writes no
/// files, so the buffer is the only place activation logs live before they are
/// posted to the server.
#[derive(Debug, Clone, Default)]
pub struct LogBuffer {
    records: Arc<Mutex<Vec<LogRecord>>>,
}

impl LogBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&self, record: LogRecord) {
        let Ok(mut records) = self.records.lock() else {
            return;
        };
        if records.len() >= MAX_LOG_RECORDS {
            records.remove(0);
        }
        records.push(record);
    }

    /// Snapshot the captured records as a JSON array string for the `logs`
    /// multipart field. Returns `"[]"` when nothing was captured so the field
    /// is always valid JSON.
    #[must_use]
    pub fn snapshot_json(&self) -> String {
        let Ok(records) = self.records.lock() else {
            return "[]".to_owned();
        };
        serde_json::to_string(&*records).unwrap_or_else(|_| "[]".to_owned())
    }
}

#[derive(Debug)]
struct BufferLayer {
    buffer: LogBuffer,
}

impl<S> Layer<S> for BufferLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &Event<'_>, _context: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.buffer.push(LogRecord {
            timestamp_utc: OffsetDateTime::from(SystemTime::now())
                .format(&Rfc3339)
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned()),
            level: metadata.level().to_string(),
            target: metadata.target().to_owned(),
            message: visitor.message,
            fields: visitor.fields,
        });
    }
}

#[derive(Debug, Default)]
struct FieldVisitor {
    message: String,
    fields: Option<String>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
            return;
        }
        let rendered = format!("{value:?}");
        match &mut self.fields {
            Some(existing) => {
                existing.push_str(", ");
                existing.push_str(field.name());
                existing.push('=');
                existing.push_str(&rendered);
            }
            None => {
                self.fields = Some(format!("{}={rendered}", field.name()));
            }
        }
    }
}

/// Initialize tracing. Production mode captures events into `buffer` only,
/// while debug mode additionally writes to stderr and the debug log file.
pub fn init_logging(log_dir: &Path, buffer: &LogBuffer) -> AppResult<Option<WorkerGuard>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let buffer_layer = BufferLayer {
        buffer: buffer.clone(),
    }
    .with_filter(filter);

    if !DEBUG {
        tracing_subscriber::registry().with(buffer_layer).init();
        return Ok(None);
    }

    std::fs::create_dir_all(log_dir).map_err(|source| io_error(log_dir, source))?;
    let file_appender = tracing_appender::rolling::never(log_dir, "activation.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(buffer_layer)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .init();

    Ok(Some(guard))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(index: usize) -> LogRecord {
        LogRecord {
            timestamp_utc: "1970-01-01T00:00:00Z".to_owned(),
            level: "INFO".to_owned(),
            target: "test".to_owned(),
            message: format!("event {index}"),
            fields: None,
        }
    }

    #[test]
    fn snapshot_json_should_return_empty_array_when_no_records() {
        assert_eq!(LogBuffer::new().snapshot_json(), "[]");
    }

    #[test]
    fn push_should_keep_only_recent_records() {
        let buffer = LogBuffer::new();
        for index in 0..(MAX_LOG_RECORDS + 5) {
            buffer.push(record(index));
        }

        let json = buffer.snapshot_json();
        assert!(!json.contains("event 0"));
        assert!(json.contains(&format!("event {}", MAX_LOG_RECORDS + 4)));
    }

    #[test]
    fn snapshot_json_should_be_valid_json_array() {
        let buffer = LogBuffer::new();
        buffer.push(record(1));

        let parsed: serde_json::Value = serde_json::from_str(&buffer.snapshot_json()).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 1);
    }
}
