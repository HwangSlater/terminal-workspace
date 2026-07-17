//! Standardized span constructors for the `Application > Command / Event /
//! Integration / Plugin` hierarchy. See `docs/05-operations/logging.md` §0.
//!
//! Defining these up front — rather than letting each subsystem invent its
//! own span shape later — means an OpenTelemetry exporter can attach to this
//! hierarchy without reworking call sites.

use crate::TraceContext;
use tracing::Span;

/// Root span opened once at process startup; every other span is a
/// descendant of this one. Generates a fresh [`TraceContext`] as the root
/// correlation id for the process.
#[must_use]
pub fn application_span() -> Span {
    let ctx = TraceContext::new();
    tracing::info_span!("application", correlation_id = %ctx.correlation_id)
}

/// Span opened when a `Command` (see `docs/02-architecture/command-model.md`) begins dispatch.
#[must_use]
pub fn command_span(name: &str, ctx: &TraceContext) -> Span {
    tracing::info_span!("command", command.name = %name, correlation_id = %ctx.correlation_id)
}

/// Span opened when an `Event` (see `docs/02-architecture/events.md`) is published or handled.
#[must_use]
pub fn event_span(kind: &str, ctx: &TraceContext) -> Span {
    tracing::info_span!("event", event.kind = %kind, correlation_id = %ctx.correlation_id)
}

/// Span opened around a call into a third-party integration adapter
/// (Slack, GitHub, Gmail, Calendar, Jira).
#[must_use]
pub fn integration_span(source: &str, ctx: &TraceContext) -> Span {
    tracing::info_span!("integration", integration.source = %source, correlation_id = %ctx.correlation_id)
}

/// Span opened around a WASM plugin guest invocation.
#[must_use]
pub fn plugin_span(plugin_id: &str, ctx: &TraceContext) -> Span {
    tracing::info_span!("plugin", plugin.id = %plugin_id, correlation_id = %ctx.correlation_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_constructors_produce_named_spans() {
        // Without an active subscriber, tracing creates disabled spans and
        // `.metadata()` returns `None` — a minimal subscriber is required
        // for this test to observe span names at all, not just to see output.
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let ctx = TraceContext::new();
        assert_eq!(application_span().metadata().unwrap().name(), "application");
        assert_eq!(
            command_span("SetPresence", &ctx).metadata().unwrap().name(),
            "command"
        );
        assert_eq!(
            event_span("SlackMessageReceived", &ctx)
                .metadata()
                .unwrap()
                .name(),
            "event"
        );
        assert_eq!(
            integration_span("slack", &ctx).metadata().unwrap().name(),
            "integration"
        );
        assert_eq!(
            plugin_span("pomodoro-timer", &ctx)
                .metadata()
                .unwrap()
                .name(),
            "plugin"
        );
    }
}
