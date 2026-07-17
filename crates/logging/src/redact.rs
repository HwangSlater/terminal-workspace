//! Secret-scrubbing writer wrapper. See `docs/05-operations/logging.md` §3 and `docs/04-extensions/security.md` §2.

use std::io;
use tracing_subscriber::fmt::MakeWriter;

const SECRET_PREFIXES: &[&str] = &["xoxb-", "ghp_"];
const REDACTED: &str = "[REDACTED_SECRET]";

/// Replace any substring beginning with a known secret prefix (e.g. `xoxb-`,
/// `ghp_`) followed by a run of token characters with `[REDACTED_SECRET]`.
#[must_use]
pub fn redact_secrets(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut i = 0;

    'outer: while i < input.len() {
        for prefix in SECRET_PREFIXES {
            if input[i..].starts_with(prefix) {
                let mut end = i + prefix.len();
                while end < input.len() {
                    let c = input[end..].chars().next().expect("valid char boundary");
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        end += c.len_utf8();
                    } else {
                        break;
                    }
                }
                result.push_str(REDACTED);
                i = end;
                continue 'outer;
            }
        }
        let c = input[i..].chars().next().expect("valid char boundary");
        result.push(c);
        i += c.len_utf8();
    }

    result
}

/// `io::Write` wrapper that scrubs secret-shaped substrings out of every
/// write before forwarding it to the wrapped writer.
pub struct RedactingWriter<W> {
    inner: W,
}

impl<W> RedactingWriter<W> {
    /// Wrap `inner`, scrubbing every write before it is forwarded.
    pub fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: io::Write> io::Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.inner.write_all(redact_secrets(&text).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// `tracing_subscriber::fmt::MakeWriter` producing a [`RedactingWriter`] over
/// stderr, for use as `fmt::layer().with_writer(RedactingMakeWriter::default())`.
#[derive(Clone, Copy, Default)]
pub struct RedactingMakeWriter;

impl<'a> MakeWriter<'a> for RedactingMakeWriter {
    type Writer = RedactingWriter<io::Stderr>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(io::stderr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn redacts_slack_bot_token() {
        let input = "leaked token xoxb-1234-abcd-EFGH here";
        assert_eq!(redact_secrets(input), "leaked token [REDACTED_SECRET] here");
    }

    #[test]
    fn redacts_github_token() {
        let input = "auth=ghp_abc123XYZ done";
        assert_eq!(redact_secrets(input), "auth=[REDACTED_SECRET] done");
    }

    #[test]
    fn leaves_ordinary_text_untouched() {
        let input = "no secrets in this log line";
        assert_eq!(redact_secrets(input), input);
    }

    #[test]
    fn redacts_multiple_occurrences() {
        let input = "xoxb-aaa and ghp_bbb both leaked";
        assert_eq!(
            redact_secrets(input),
            "[REDACTED_SECRET] and [REDACTED_SECRET] both leaked"
        );
    }

    #[test]
    fn redacting_writer_masks_before_reaching_inner() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut writer = RedactingWriter::new(&mut buf);
            writer.write_all(b"token xoxb-abc123 leaked").unwrap();
        }
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "token [REDACTED_SECRET] leaked"
        );
    }
}
