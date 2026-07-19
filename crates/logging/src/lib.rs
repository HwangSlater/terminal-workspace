//! System Tracing and Logging initializer.

pub mod buffer;
pub mod redact;
pub mod spans;

pub use buffer::{LogBuffer, LogBufferMakeWriter};
pub use redact::{redact_secrets, RedactingMakeWriter, RedactingWriter};

use common::{Result, WorkspaceError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Trace Context carrying Correlation ID across asynchronous boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceContext {
    /// Global transaction correlation identifier.
    pub correlation_id: String,
    /// Parent span trace identifier.
    pub span_id: String,
}

impl TraceContext {
    /// Create new context with generated correlation ID.
    #[must_use]
    pub fn new() -> Self {
        Self {
            correlation_id: uuid::Uuid::new_v4().to_string(),
            span_id: "".into(),
        }
    }
}

impl Default for TraceContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve the OS-standard data directory for rotating log files, mirroring
/// `storage::standard_db_path`'s own resolution (`AppData/Local` on
/// Windows, `.local/share` elsewhere), one level down in a `logs`
/// subdirectory since a daily-rotating appender writes multiple files.
fn standard_log_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    let mut path = PathBuf::from(home);
    if cfg!(windows) {
        path.push("AppData");
        path.push("Local");
    } else {
        path.push(".local");
        path.push("share");
    }
    path.push("terminal-workspace");
    path.push("logs");
    path
}

/// Configure system-wide structured logger format. Log output is routed
/// through [`RedactingMakeWriter`] so secret-shaped substrings never reach
/// disk (see `docs/04-extensions/security.md` §2). Also feeds a second,
/// compact-formatted copy of every event into the returned [`LogBuffer`]
/// (through the same redaction wrapper, `step17.md` Decision 1) for the
/// TUI's live log panel -- both outputs share one `EnvFilter`, so
/// `log_level`/`RUST_LOG` controls what appears in each identically
/// (`step17.md` Decision 4).
///
/// The formatted-text copy writes to a daily-rotating file under
/// [`standard_log_dir`], not stderr (`step35.md`) -- stderr and the TUI's
/// alternate screen share the same physical terminal, so any log line
/// emitted while the TUI owns the screen (e.g. a background poller's
/// `WARN` mid-session) used to tear straight through the rendered frame.
/// The returned [`WorkerGuard`] must be kept alive for the process's
/// whole lifetime (its `Drop` flushes buffered lines) -- letting it drop
/// early silently truncates the log file, so the caller should bind it
/// with a name like `_log_guard` at the top of `main` rather than
/// discarding it.
pub fn init_logger(log_level: &str) -> Result<(Arc<LogBuffer>, WorkerGuard)> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));
    let log_buffer = LogBuffer::new(buffer::CAPACITY);

    let log_dir = standard_log_dir();
    std::fs::create_dir_all(&log_dir).map_err(|e| {
        WorkspaceError::Internal(format!(
            "failed to create log directory {}: {e}",
            log_dir.display()
        ))
    })?;
    let file_appender = tracing_appender::rolling::daily(&log_dir, "app.log");
    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(RedactingMakeWriter::new(non_blocking_file)),
        )
        .with(
            fmt::layer()
                .compact()
                .with_ansi(false)
                .with_target(false)
                .with_writer(LogBufferMakeWriter::new(Arc::clone(&log_buffer))),
        )
        .init();

    info!(
        "Structured logging subsystem initialized with level: {} (file logs: {})",
        log_level,
        log_dir.display()
    );
    Ok((log_buffer, guard))
}
