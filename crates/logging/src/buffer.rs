//! Bounded ring buffer of formatted log lines, fed by a `tracing_subscriber`
//! layer and read by the TUI's bottom "로그" dock (`step17.md`).

use crate::redact::RedactingWriter;
use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

/// Fixed capacity kept in [`LogBuffer`] before the oldest line is dropped
/// (`step17.md` Decision 2) -- enough scrollback to be useful without
/// unbounded growth over a long-running session.
pub const CAPACITY: usize = 200;

/// Holds the most recent [`CAPACITY`] formatted log lines. Cheap to clone
/// (it's an `Arc` internally via [`LogBuffer::new`]); `snapshot()` is a
/// synchronous, non-blocking read suitable for calling once per render
/// frame.
pub struct LogBuffer {
    lines: Mutex<VecDeque<String>>,
    capacity: usize,
}

impl LogBuffer {
    /// Create a new buffer wrapped in `Arc`, ready to hand to both
    /// [`crate::init_logger`]'s writer and the TUI's render loop.
    #[must_use]
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            lines: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        })
    }

    fn push_line(&self, line: &str) {
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if lines.len() >= self.capacity {
            lines.pop_front();
        }
        lines.push_back(line.to_string());
    }

    /// A point-in-time copy of every buffered line, oldest first.
    #[must_use]
    pub fn snapshot(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }
}

/// `io::Write` that splits each write into lines and pushes non-empty ones
/// into a [`LogBuffer`]. `pub` only because [`MakeWriter::Writer`] requires
/// it; the inner field stays private, so a caller outside this module can
/// still only get one wrapped in [`RedactingWriter`] via
/// [`LogBufferMakeWriter`] -- never bare, so redaction can't be
/// accidentally bypassed.
pub struct LogBufferWriter(Arc<LogBuffer>);

impl io::Write for LogBufferWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        for line in text.lines() {
            if !line.is_empty() {
                self.0.push_line(line);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `tracing_subscriber::fmt::MakeWriter` producing a [`RedactingWriter`]
/// over a [`LogBuffer`] -- the same secret-scrubbing wrapper the console
/// writer ([`crate::RedactingMakeWriter`]) already uses, just pointed at
/// the buffer instead of stderr (`step17.md` Decision 1: whatever feeds
/// the TUI panel must not bypass redaction).
#[derive(Clone)]
pub struct LogBufferMakeWriter(Arc<LogBuffer>);

impl LogBufferMakeWriter {
    /// Wrap `buffer` for use as `fmt::layer().with_writer(..)`.
    #[must_use]
    pub fn new(buffer: Arc<LogBuffer>) -> Self {
        Self(buffer)
    }
}

impl<'a> MakeWriter<'a> for LogBufferMakeWriter {
    type Writer = RedactingWriter<LogBufferWriter>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(LogBufferWriter(Arc::clone(&self.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn snapshot_returns_lines_in_insertion_order() {
        let buffer = LogBuffer::new(10);
        buffer.push_line("first");
        buffer.push_line("second");
        buffer.push_line("third");

        assert_eq!(buffer.snapshot(), vec!["first", "second", "third"]);
    }

    #[test]
    fn pushing_past_capacity_drops_the_oldest_line_not_the_newest() {
        let buffer = LogBuffer::new(3);
        buffer.push_line("one");
        buffer.push_line("two");
        buffer.push_line("three");
        buffer.push_line("four");

        assert_eq!(buffer.snapshot(), vec!["two", "three", "four"]);
    }

    #[test]
    fn empty_buffer_snapshot_is_empty() {
        let buffer = LogBuffer::new(10);
        assert!(buffer.snapshot().is_empty());
    }

    #[test]
    fn make_writer_redacts_secrets_before_they_reach_the_buffer() {
        let buffer = LogBuffer::new(10);
        let make_writer = LogBufferMakeWriter::new(Arc::clone(&buffer));

        let mut writer = make_writer.make_writer();
        writer
            .write_all(b"token leaked: xoxb-abc123 in this line\n")
            .unwrap();

        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert!(!snapshot[0].contains("xoxb-abc123"));
        assert!(snapshot[0].contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn a_multi_line_write_is_split_into_separate_buffer_entries() {
        let buffer = LogBuffer::new(10);
        let make_writer = LogBufferMakeWriter::new(Arc::clone(&buffer));

        let mut writer = make_writer.make_writer();
        writer.write_all(b"line one\nline two\n").unwrap();

        assert_eq!(buffer.snapshot(), vec!["line one", "line two"]);
    }
}
