//! System Tracing and Logging initializer.

pub mod redact;
pub mod spans;

pub use redact::{redact_secrets, RedactingMakeWriter, RedactingWriter};

use common::Result;
use serde::{Deserialize, Serialize};
use tracing::info;
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

/// Configure system-wide structured logger format. Log output is routed
/// through [`RedactingMakeWriter`] so secret-shaped substrings never reach
/// stderr (see `docs/04-extensions/security.md` §2).
pub fn init_logger(log_level: &str) -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(RedactingMakeWriter))
        .init();

    info!(
        "Structured logging subsystem initialized with level: {}",
        log_level
    );
    Ok(())
}
