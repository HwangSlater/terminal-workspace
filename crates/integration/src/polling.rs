//! Shared polling-adapter machinery: the consecutive-failure state machine
//! (`docs/04-extensions/integration-contract.md` §2.1) and rate-limit
//! header parsing. Fully generic across adapters — hoisted out of
//! `slack.rs` while building `github.rs` (`step10.md`) once it became clear
//! a second adapter needed the exact same logic, not a Slack-specific
//! variant of it.

use crate::ConnectionStatus;
use events::IntegrationConnectionStatus;

pub(crate) const RECONNECTING_THRESHOLD: u32 = 5;
pub(crate) const FAILED_THRESHOLD: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PollResult {
    Success,
    RateLimited,
    Failure,
}

pub(crate) fn max_option(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

/// Pure state transition per `integration-contract.md` §2.1: a success
/// always resets to `Connected`; a rate-limited cycle is a no-op (not
/// counted as a failure); a failure only changes status once the
/// consecutive-failure count crosses the `Reconnecting`/`Failed` thresholds.
pub(crate) fn next_status(
    prev: &ConnectionStatus,
    consecutive_failures: u32,
    result: PollResult,
) -> (u32, ConnectionStatus) {
    match result {
        PollResult::Success => (0, ConnectionStatus::Connected),
        PollResult::RateLimited => (consecutive_failures, prev.clone()),
        PollResult::Failure => {
            let failures = consecutive_failures + 1;
            let status = if failures >= FAILED_THRESHOLD {
                ConnectionStatus::Failed(format!("{failures} consecutive poll failures"))
            } else if failures >= RECONNECTING_THRESHOLD {
                ConnectionStatus::Reconnecting
            } else {
                prev.clone()
            };
            (failures, status)
        }
    }
}

/// Maps this crate's own `ConnectionStatus` to the structurally-identical
/// but separately-defined `events::IntegrationConnectionStatus` (ADR-0016
/// explains why `crates/events` can't just re-export this crate's type).
pub(crate) fn to_event_status(status: &ConnectionStatus) -> IntegrationConnectionStatus {
    match status {
        ConnectionStatus::Disconnected => IntegrationConnectionStatus::Disconnected,
        ConnectionStatus::Connecting => IntegrationConnectionStatus::Connecting,
        ConnectionStatus::Connected => IntegrationConnectionStatus::Connected,
        ConnectionStatus::Reconnecting => IntegrationConnectionStatus::Reconnecting,
        ConnectionStatus::Failed(reason) => IntegrationConnectionStatus::Failed(reason.clone()),
    }
}

pub(crate) fn retry_after_seconds(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_through_fourth_consecutive_failures_do_not_change_status() {
        let mut status = ConnectionStatus::Connected;
        let mut failures = 0;
        for _ in 0..4 {
            let (f, s) = next_status(&status, failures, PollResult::Failure);
            failures = f;
            status = s;
        }
        assert_eq!(failures, 4);
        assert_eq!(status, ConnectionStatus::Connected);
    }

    #[test]
    fn fifth_consecutive_failure_moves_to_reconnecting() {
        let (failures, status) = next_status(&ConnectionStatus::Connected, 4, PollResult::Failure);
        assert_eq!(failures, 5);
        assert_eq!(status, ConnectionStatus::Reconnecting);
    }

    #[test]
    fn tenth_consecutive_failure_moves_to_failed() {
        let (failures, status) =
            next_status(&ConnectionStatus::Reconnecting, 9, PollResult::Failure);
        assert_eq!(failures, 10);
        assert!(matches!(status, ConnectionStatus::Failed(_)));
    }

    #[test]
    fn success_after_failures_resets_the_counter() {
        let (failures, status) =
            next_status(&ConnectionStatus::Reconnecting, 7, PollResult::Success);
        assert_eq!(failures, 0);
        assert_eq!(status, ConnectionStatus::Connected);
    }

    #[test]
    fn rate_limited_cycle_does_not_count_as_a_failure() {
        let (failures, status) =
            next_status(&ConnectionStatus::Connected, 3, PollResult::RateLimited);
        assert_eq!(failures, 3);
        assert_eq!(status, ConnectionStatus::Connected);
    }

    #[test]
    fn to_event_status_maps_every_variant() {
        assert_eq!(
            to_event_status(&ConnectionStatus::Disconnected),
            IntegrationConnectionStatus::Disconnected
        );
        assert_eq!(
            to_event_status(&ConnectionStatus::Connecting),
            IntegrationConnectionStatus::Connecting
        );
        assert_eq!(
            to_event_status(&ConnectionStatus::Connected),
            IntegrationConnectionStatus::Connected
        );
        assert_eq!(
            to_event_status(&ConnectionStatus::Reconnecting),
            IntegrationConnectionStatus::Reconnecting
        );
        assert_eq!(
            to_event_status(&ConnectionStatus::Failed("x".into())),
            IntegrationConnectionStatus::Failed("x".into())
        );
    }

    #[test]
    fn parses_retry_after_header() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "42".parse().unwrap());
        assert_eq!(retry_after_seconds(&headers), Some(42));
    }

    #[test]
    fn missing_retry_after_header_is_none() {
        let headers = reqwest::header::HeaderMap::new();
        assert_eq!(retry_after_seconds(&headers), None);
    }
}
