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

use crate::config::{APP_VERSION, DEBUG};
use crate::error::{AppResult, io_error};

/// Keeps the activation log payload bounded so a long retry loop cannot grow
/// the multipart body without limit. Only the most recent records are sent.
pub const MAX_LOG_RECORDS: usize = 200;

/// One structured tracing event captured for the server-side `logs` field.
///
/// Every record is self-contained: it repeats the build version, the run id,
/// and the activation attempt so production traces can be grouped without
/// correlating separate payload fields.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogRecord {
    pub timestamp_utc: String,
    pub level: String,
    pub target: String,
    pub message: String,
    pub app_version: String,
    pub session_id: String,
    pub attempt: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<String>,
}

/// Per-run trace context stamped onto every captured record. The session id is
/// generated once per process start, so records from the same activation run can
/// be grouped; the attempt counter is updated before each POST.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RunContext {
    session_id: String,
    attempt: u64,
}

/// Shared in-memory ring buffer of structured logs. Production mode writes no
/// files, so the buffer is the only place activation logs live before they are
/// posted to the server.
#[derive(Debug, Clone)]
pub struct LogBuffer {
    records: Arc<Mutex<Vec<LogRecord>>>,
    run: Arc<Mutex<RunContext>>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl LogBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            run: Arc::new(Mutex::new(RunContext {
                session_id: new_session_id(),
                attempt: 0,
            })),
        }
    }

    /// Trace id for the current process run. Stable for the whole run and
    /// repeated on every record.
    #[must_use]
    pub fn session_id(&self) -> String {
        let Ok(run) = self.run.lock() else {
            return String::new();
        };
        run.session_id.clone()
    }

    /// Record which activation attempt is in flight. Events captured after this
    /// call (including the retry warning for the failed attempt) carry the new
    /// number. `0` means the pre-POST collection phase.
    pub fn set_attempt(&self, attempt: u64) {
        if let Ok(mut run) = self.run.lock() {
            run.attempt = attempt;
        }
    }

    /// Attempt number currently stamped onto new records.
    #[must_use]
    #[cfg(test)]
    pub fn current_attempt(&self) -> u64 {
        let Ok(run) = self.run.lock() else {
            return 0;
        };
        run.attempt
    }

    /// Store one captured event, stamping the current run context onto it. This
    /// is the only place records are written, so every shipped record carries
    /// the app version, session id, and attempt.
    fn capture(
        &self,
        timestamp_utc: String,
        level: String,
        target: String,
        message: String,
        fields: Option<String>,
    ) {
        let (session_id, attempt) = match self.run.lock() {
            Ok(run) => (run.session_id.clone(), run.attempt),
            Err(_) => (String::new(), 0),
        };
        self.push(LogRecord {
            timestamp_utc,
            level,
            target,
            message,
            app_version: APP_VERSION.to_owned(),
            session_id,
            attempt,
            fields,
        });
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

/// Short random identifier shared by every record of one process run.
fn new_session_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
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
        self.buffer.capture(
            OffsetDateTime::from(SystemTime::now())
                .format(&Rfc3339)
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned()),
            metadata.level().to_string(),
            metadata.target().to_owned(),
            visitor.message,
            visitor.fields,
        );
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
            app_version: APP_VERSION.to_owned(),
            session_id: buffer_session(),
            attempt: 0,
            fields: None,
        }
    }

    fn buffer_session() -> String {
        "session".to_owned()
    }

    /// Capture an event through the same path the tracing layer uses, so tests
    /// exercise the real context stamping.
    fn capture(buffer: &LogBuffer, message: &str) {
        buffer.capture(
            "1970-01-01T00:00:00Z".to_owned(),
            "INFO".to_owned(),
            "test".to_owned(),
            message.to_owned(),
            None,
        );
    }

    #[test]
    fn snapshot_json_should_return_empty_array_when_no_records() {
        assert_eq!(LogBuffer::new().snapshot_json(), "[]");
    }

    #[test]
    fn snapshot_json_should_include_trace_context_per_record() {
        let buffer = LogBuffer::new();
        buffer.set_attempt(3);
        capture(&buffer, "activation attempt");

        let parsed: serde_json::Value = serde_json::from_str(&buffer.snapshot_json()).unwrap();
        let first = &parsed.as_array().unwrap()[0];

        assert_eq!(first["app_version"], APP_VERSION);
        assert_eq!(first["attempt"], 3);
        assert_eq!(first["session_id"], buffer.session_id());
    }

    #[test]
    fn session_id_should_be_stable_and_shared_across_clones() {
        let buffer = LogBuffer::new();
        let clone = buffer.clone();

        assert!(!buffer.session_id().is_empty());
        assert_eq!(buffer.session_id(), clone.session_id());
    }

    #[test]
    fn set_attempt_should_be_visible_to_clones() {
        let buffer = LogBuffer::new();
        let clone = buffer.clone();
        buffer.set_attempt(7);

        assert_eq!(clone.current_attempt(), 7);
    }

    #[test]
    fn capture_should_default_attempt_to_zero_before_first_post() {
        let buffer = LogBuffer::new();
        capture(&buffer, "startup");

        let parsed: serde_json::Value = serde_json::from_str(&buffer.snapshot_json()).unwrap();

        assert_eq!(parsed.as_array().unwrap()[0]["attempt"], 0);
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
