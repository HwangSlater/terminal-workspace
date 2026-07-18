//! Google Calendar (iCal secret-URL) adapter. See
//! `docs/04-extensions/integrations/calendar.md` for the endpoint/mapping
//! spec and `step12.md` for the design decisions (secret iCal URL over
//! OAuth, a real `RRULE` expansion dependency over hand-rolled parsing —
//! recurring events are the common case, not the exception).

use crate::polling::{next_status, retry_after_seconds, to_event_status, PollResult};
use crate::{ConnectionStatus, IntegrationAdapter, IntegrationConnector};
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use common::{Result, WorkspaceError};
use domain::{IntegrationSource, NotificationId, NotificationItem, PriorityLevel};
use events::{Event, EventBus, IntegrationConnectionStatus};
use ical::parser::ical::component::IcalEvent;
use ical::parser::Component;
use ical::property::Property;
use ical::IcalParser;
use rrule::RRuleSet;
use secrecy::ExposeSecret;
use secrets::{SecretProvider, SecretWriter};
use std::collections::HashSet;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

const CALENDAR_URL_KEY: &str = "CALENDAR_ICAL_URL";

/// Fixed namespace for deriving deterministic per-occurrence notification
/// ids from `"{event UID}#{occurrence start epoch millis}"` (UUIDv5) — the
/// same occurrence re-polled upserts instead of duplicating. Distinct from
/// Slack's/GitHub's namespaces so the three integrations can never collide.
const CALENDAR_OCCURRENCE_ID_NAMESPACE: Uuid = Uuid::from_bytes([
    0x9a, 0x1c, 0x2e, 0x3f, 0x4a, 0x5b, 0x46, 0x71, 0x88, 0x92, 0xa3, 0xb4, 0xc5, 0xd6, 0xe7, 0xf8,
]);

/// Static per-integration configuration (non-secret) — the secret iCal URL
/// itself comes from `SecretProviderChain`, never from this struct.
#[derive(Debug, Clone)]
pub struct CalendarConfig {
    /// Only occurrences starting within this many hours from "now" become
    /// a notification — a reminder feature, not a full calendar dump.
    pub lookahead_hours: u64,
    /// Seconds between poll cycles.
    pub sync_interval_secs: u64,
}

struct AdapterState {
    status: ConnectionStatus,
    consecutive_failures: u32,
    token: Option<String>,
}

/// Google Calendar adapter, driven by a calendar's secret iCal feed URL
/// rather than OAuth (`step12.md` Decision 1). Polls on an interval —
/// same rationale as `SlackAdapter`/`GitHubAdapter`. No `Picker` impl: the
/// secret-URL model has no "list my calendars" discovery call, so there is
/// nothing for a picker to list (`step12.md` Decision 1's consequence).
pub struct CalendarAdapter {
    config: Arc<RwLock<CalendarConfig>>,
    http: reqwest::Client,
    state: Arc<RwLock<AdapterState>>,
    seen_occurrences: Arc<Mutex<HashSet<(String, i64)>>>,
    poll_task: Mutex<Option<JoinHandle<()>>>,
    secret_writer: Arc<dyn SecretWriter>,
}

impl CalendarAdapter {
    /// Create a new adapter. Call [`IntegrationAdapter::initialize`] before
    /// [`IntegrationAdapter::start`]. `secret_writer` is where
    /// [`IntegrationConnector::connect`] persists a URL entered through the
    /// setup UI — normally a `SecretProviderChain`.
    #[must_use]
    pub fn new(config: CalendarConfig, secret_writer: Arc<dyn SecretWriter>) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            http: reqwest::Client::new(),
            state: Arc::new(RwLock::new(AdapterState {
                status: ConnectionStatus::Disconnected,
                consecutive_failures: 0,
                token: None,
            })),
            seen_occurrences: Arc::new(Mutex::new(HashSet::new())),
            poll_task: Mutex::new(None),
            secret_writer,
        }
    }
}

#[async_trait]
impl IntegrationAdapter for CalendarAdapter {
    async fn initialize(&self, secret_provider: &dyn SecretProvider) -> Result<()> {
        let token = secret_provider.get_secret(CALENDAR_URL_KEY).await?;
        let mut state = self.state.write().await;
        match token {
            Some(secret) => {
                state.token = Some(secret.expose_secret().to_string());
                state.status = ConnectionStatus::Connecting;
            }
            None => {
                state.token = None;
                state.status = ConnectionStatus::Disconnected;
            }
        }
        Ok(())
    }

    async fn start(&self, event_bus: Arc<dyn EventBus>) -> Result<()> {
        let url = self.state.read().await.token.clone();
        let Some(url) = url else {
            tracing::info!(
                "Calendar adapter has no credential; staying Disconnected (Zero Configuration)."
            );
            return Ok(());
        };

        let poller = CalendarPoller {
            http: self.http.clone(),
            config: self.config.read().await.clone(),
            state: Arc::clone(&self.state),
            seen_occurrences: Arc::clone(&self.seen_occurrences),
        };
        let handle = tokio::spawn(poller.run_loop(event_bus, url));
        *self.poll_task.lock().await = Some(handle);
        Ok(())
    }

    async fn health_check(&self) -> Result<ConnectionStatus> {
        Ok(self.state.read().await.status.clone())
    }

    async fn shutdown(&self) -> Result<()> {
        if let Some(handle) = self.poll_task.lock().await.take() {
            handle.abort();
        }
        Ok(())
    }
}

#[async_trait]
impl IntegrationConnector for CalendarAdapter {
    async fn connect(&self, event_bus: Arc<dyn EventBus>, token: String) -> Result<()> {
        self.secret_writer
            .set_secret(CALENDAR_URL_KEY, &token)
            .await?;
        {
            let mut state = self.state.write().await;
            state.token = Some(token);
            state.status = ConnectionStatus::Connecting;
            state.consecutive_failures = 0;
        }
        let _ = event_bus
            .publish(Event::IntegrationStatusChanged {
                source: IntegrationSource::Calendar,
                status: IntegrationConnectionStatus::Connecting,
            })
            .await;
        self.shutdown().await?;
        self.start(event_bus).await
    }
}

struct CalendarPoller {
    http: reqwest::Client,
    config: CalendarConfig,
    state: Arc<RwLock<AdapterState>>,
    seen_occurrences: Arc<Mutex<HashSet<(String, i64)>>>,
}

impl CalendarPoller {
    async fn poll_once(
        &self,
        event_bus: &Arc<dyn EventBus>,
        url: &str,
    ) -> (PollResult, Option<u64>) {
        let response = match self.http.get(url).send().await {
            Ok(r) => r,
            Err(_) => return (PollResult::Failure, None),
        };

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return (
                PollResult::RateLimited,
                retry_after_seconds(response.headers()),
            );
        }
        if !response.status().is_success() {
            return (PollResult::Failure, None);
        }
        let body = match response.text().await {
            Ok(b) => b,
            Err(_) => return (PollResult::Failure, None),
        };

        let calendar = match parse_first_calendar(&body) {
            Ok(cal) => cal,
            Err(_) => return (PollResult::Failure, None),
        };

        let now = Utc::now();
        let horizon =
            now + ChronoDuration::hours(self.config.lookahead_hours.min(i64::MAX as u64) as i64);
        let mut any_failure = false;

        for event in &calendar.events {
            let occurrences = match expand_occurrences(event, now, horizon) {
                Ok(occurrences) => occurrences,
                // A single malformed VEVENT (unexpected property shape,
                // unsupported RRULE quirk) shouldn't take the whole feed's
                // status down -- skip just that event, same as Slack's
                // "missing display name falls back to raw id" degradation.
                Err(_) => continue,
            };

            for occurrence in occurrences {
                let key = (event_uid(event), occurrence.timestamp_millis());
                let already_seen = self.seen_occurrences.lock().await.contains(&key);
                if already_seen {
                    continue;
                }
                let item = map_occurrence(event, occurrence);
                if event_bus
                    .publish(Event::CalendarReminderTriggered(item))
                    .await
                    .is_err()
                {
                    any_failure = true;
                }
                self.seen_occurrences.lock().await.insert(key);
            }
        }

        if any_failure {
            (PollResult::Failure, None)
        } else {
            (PollResult::Success, None)
        }
    }

    async fn run_loop(self, event_bus: Arc<dyn EventBus>, url: String) {
        let base_interval = Duration::from_secs(self.config.sync_interval_secs.max(1));
        loop {
            let (result, retry_after) = self.poll_once(&event_bus, &url).await;

            let (status, prev_status) = {
                let mut state = self.state.write().await;
                let prev_status = state.status.clone();
                let (failures, status) =
                    next_status(&prev_status, state.consecutive_failures, result);
                state.consecutive_failures = failures;
                state.status = status.clone();
                (status, prev_status)
            };

            if status != prev_status {
                let _ = event_bus
                    .publish(Event::IntegrationStatusChanged {
                        source: IntegrationSource::Calendar,
                        status: to_event_status(&status),
                    })
                    .await;
            }

            if let ConnectionStatus::Failed(reason) = &status {
                if !matches!(prev_status, ConnectionStatus::Failed(_)) {
                    let _ = event_bus
                        .publish(Event::SystemAlert(format!(
                            "Calendar integration failed: {reason}"
                        )))
                        .await;
                }
            }

            let wait = match (result, retry_after) {
                (PollResult::RateLimited, Some(secs)) => Duration::from_secs(secs),
                _ => base_interval,
            };
            tokio::time::sleep(wait).await;
        }
    }
}

/// Parses the first `VCALENDAR` component out of `body`. A secret iCal
/// feed URL always returns exactly one.
fn parse_first_calendar(
    body: &str,
) -> std::result::Result<ical::parser::ical::component::IcalCalendar, WorkspaceError> {
    let mut parser = IcalParser::new(BufReader::new(body.as_bytes()));
    match parser.next() {
        Some(Ok(calendar)) => Ok(calendar),
        Some(Err(e)) => Err(WorkspaceError::Integration(format!(
            "calendar feed parse failed: {e}"
        ))),
        None => Err(WorkspaceError::Integration(
            "calendar feed contained no VCALENDAR".into(),
        )),
    }
}

fn event_uid(event: &IcalEvent) -> String {
    event
        .get_property("UID")
        .and_then(|p| p.value.clone())
        .unwrap_or_default()
}

/// Reconstructs a `Property` back into the raw `NAME[;PARAM=V1,V2]:VALUE`
/// line shape `RRuleSet::from_str` expects — `ical` gives us parsed parts,
/// not the original line text.
fn property_to_ical_line(prop: &Property) -> String {
    let mut line = prop.name.clone();
    if let Some(params) = &prop.params {
        for (key, values) in params {
            line.push(';');
            line.push_str(key);
            line.push('=');
            line.push_str(&values.join(","));
        }
    }
    line.push(':');
    line.push_str(prop.value.as_deref().unwrap_or(""));
    line
}

/// Occurrences of `event` starting within `[after, before)`. Always goes
/// through `RRuleSet` (`rrule` crate, `step12.md` Decision 2) — even a
/// non-recurring event is a valid one-rule-less `RRuleSet` (a bare
/// `DTSTART`), so there's a single code path rather than a hand-rolled
/// fallback for the "no RRULE" case.
fn expand_occurrences(
    event: &IcalEvent,
    after: chrono::DateTime<Utc>,
    before: chrono::DateTime<Utc>,
) -> std::result::Result<Vec<chrono::DateTime<rrule::Tz>>, WorkspaceError> {
    let dtstart = event
        .get_property("DTSTART")
        .ok_or_else(|| WorkspaceError::Integration("VEVENT missing DTSTART".into()))?;

    let mut lines = vec![property_to_ical_line(dtstart)];
    if let Some(rrule) = event.get_property("RRULE") {
        lines.push(property_to_ical_line(rrule));
    } else {
        // `RRuleSet`'s iterator only ever yields RRULE/RDATE-derived
        // occurrences -- a DTSTART with neither present yields *zero*
        // occurrences, not an implicit one at DTSTART itself (confirmed by
        // reading rrule 0.14's `RRuleSetIter::into_iter`, which builds its
        // queue solely from `self.rrule`/`self.rdate`). Without this, every
        // non-recurring event -- the common case -- would silently never
        // produce a reminder. Inject DTSTART as an explicit RDATE so a
        // plain, non-recurring event still yields its one occurrence.
        let mut synthetic_rdate = dtstart.clone();
        synthetic_rdate.name = "RDATE".to_string();
        lines.push(property_to_ical_line(&synthetic_rdate));
    }
    for exdate in event.properties.iter().filter(|p| p.name == "EXDATE") {
        lines.push(property_to_ical_line(exdate));
    }

    let set: RRuleSet = lines
        .join("\n")
        .parse()
        .map_err(|e| WorkspaceError::Integration(format!("RRULE parse failed: {e}")))?;

    let after_tz = after.with_timezone(&rrule::Tz::UTC);
    let before_tz = before.with_timezone(&rrule::Tz::UTC);
    let result = set.after(after_tz).before(before_tz).all(366);
    Ok(result.dates)
}

fn map_occurrence(event: &IcalEvent, occurrence: chrono::DateTime<rrule::Tz>) -> NotificationItem {
    let summary = event
        .get_property("SUMMARY")
        .and_then(|p| p.value.clone())
        .unwrap_or_else(|| "(제목 없음)".to_string());
    let location = event.get_property("LOCATION").and_then(|p| p.value.clone());
    let action_link = event.get_property("URL").and_then(|p| p.value.clone());

    NotificationItem {
        id: NotificationId(Uuid::new_v5(
            &CALENDAR_OCCURRENCE_ID_NAMESPACE,
            format!("{}#{}", event_uid(event), occurrence.timestamp_millis()).as_bytes(),
        )),
        source: IntegrationSource::Calendar,
        title: summary,
        body: location.unwrap_or_default(),
        timestamp_ms: u64::try_from(occurrence.timestamp_millis().max(0)).unwrap_or(0),
        priority: PriorityLevel::Medium,
        is_read: false,
        action_link,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoneProvider;

    #[async_trait]
    impl SecretProvider for NoneProvider {
        async fn get_secret(&self, _key: &str) -> Result<Option<secrecy::SecretString>> {
            Ok(None)
        }
    }

    struct FixedProvider(&'static str);

    #[async_trait]
    impl SecretProvider for FixedProvider {
        async fn get_secret(&self, _key: &str) -> Result<Option<secrecy::SecretString>> {
            Ok(Some(secrecy::SecretString::from(self.0.to_string())))
        }
    }

    #[derive(Default)]
    struct RecordingWriter {
        written: tokio::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl SecretWriter for RecordingWriter {
        async fn set_secret(&self, key: &str, value: &str) -> Result<()> {
            self.written
                .lock()
                .await
                .push((key.to_string(), value.to_string()));
            Ok(())
        }
    }

    fn test_config() -> CalendarConfig {
        CalendarConfig {
            lookahead_hours: 24,
            sync_interval_secs: 300,
        }
    }

    fn test_adapter() -> (CalendarAdapter, Arc<RecordingWriter>) {
        let writer = Arc::new(RecordingWriter::default());
        let adapter =
            CalendarAdapter::new(test_config(), Arc::clone(&writer) as Arc<dyn SecretWriter>);
        (adapter, writer)
    }

    #[tokio::test]
    async fn initialize_with_no_url_reports_disconnected_not_error() {
        let (adapter, _writer) = test_adapter();
        let result = adapter.initialize(&NoneProvider).await;
        assert!(result.is_ok());
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Disconnected
        );
    }

    #[tokio::test]
    async fn initialize_with_url_reports_connecting() {
        let (adapter, _writer) = test_adapter();
        adapter
            .initialize(&FixedProvider(
                "https://calendar.google.com/calendar/ical/x/private-y/basic.ics",
            ))
            .await
            .unwrap();
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Connecting
        );
    }

    #[tokio::test]
    async fn connect_persists_the_url_and_transitions_to_connecting() {
        let (adapter, writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;

        adapter
            .connect(
                Arc::clone(&event_bus),
                "https://example.com/cal.ics".to_string(),
            )
            .await
            .unwrap();

        assert_eq!(
            writer.written.lock().await.as_slice(),
            [(
                CALENDAR_URL_KEY.to_string(),
                "https://example.com/cal.ics".to_string()
            )]
        );
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Connecting
        );
    }

    #[tokio::test]
    async fn connect_publishes_an_integration_status_changed_event() {
        let (adapter, _writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8));
        let mut rx = event_bus.subscribe();

        adapter
            .connect(
                Arc::clone(&event_bus) as Arc<dyn EventBus>,
                "https://example.com/cal.ics".to_string(),
            )
            .await
            .unwrap();

        let event = rx
            .try_recv()
            .expect("connect() must publish a status event");
        match event {
            Event::IntegrationStatusChanged { source, status } => {
                assert_eq!(source, IntegrationSource::Calendar);
                assert_eq!(status, IntegrationConnectionStatus::Connecting);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_without_credential_does_not_spawn_a_poll_loop() {
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;
        let (adapter, _writer) = test_adapter();
        adapter.initialize(&NoneProvider).await.unwrap();
        assert!(adapter.start(event_bus).await.is_ok());
    }

    const NON_RECURRING_ICS: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:single-event-1\r\n\
SUMMARY:Design Review\r\n\
DTSTART:20250101T090000Z\r\n\
DTEND:20250101T100000Z\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    const RECURRING_ICS: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:standup-1\r\n\
SUMMARY:Daily Standup\r\n\
DTSTART:20250101T090000Z\r\n\
DTEND:20250101T091500Z\r\n\
RRULE:FREQ=DAILY;COUNT=10\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn parses_a_single_vcalendar_with_one_vevent() {
        let calendar = parse_first_calendar(NON_RECURRING_ICS).unwrap();
        assert_eq!(calendar.events.len(), 1);
        assert_eq!(
            calendar.events[0].get_property("SUMMARY").unwrap().value,
            Some("Design Review".to_string())
        );
    }

    #[test]
    fn a_non_recurring_event_yields_exactly_one_occurrence_in_a_wide_enough_window() {
        let calendar = parse_first_calendar(NON_RECURRING_ICS).unwrap();
        let event = &calendar.events[0];
        let after = chrono::DateTime::parse_from_rfc3339("2024-12-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let before = chrono::DateTime::parse_from_rfc3339("2025-02-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let occurrences = expand_occurrences(event, after, before).unwrap();
        assert_eq!(occurrences.len(), 1);
    }

    #[test]
    fn a_non_recurring_event_outside_the_window_yields_no_occurrences() {
        let calendar = parse_first_calendar(NON_RECURRING_ICS).unwrap();
        let event = &calendar.events[0];
        // Window entirely before the event's DTSTART.
        let after = chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let before = chrono::DateTime::parse_from_rfc3339("2020-02-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let occurrences = expand_occurrences(event, after, before).unwrap();
        assert!(occurrences.is_empty());
    }

    #[test]
    fn a_daily_recurring_event_expands_to_multiple_occurrences_in_window() {
        let calendar = parse_first_calendar(RECURRING_ICS).unwrap();
        let event = &calendar.events[0];
        let after = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let before = chrono::DateTime::parse_from_rfc3339("2025-01-04T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let occurrences = expand_occurrences(event, after, before).unwrap();
        // Jan 1, 2, 3 fall in [after, before) out of the 10-count series.
        assert_eq!(occurrences.len(), 3);
    }

    #[test]
    fn maps_occurrence_to_notification_item() {
        let calendar = parse_first_calendar(NON_RECURRING_ICS).unwrap();
        let event = &calendar.events[0];
        let occurrence = chrono::DateTime::parse_from_rfc3339("2025-01-01T09:00:00Z")
            .unwrap()
            .with_timezone(&rrule::Tz::UTC);
        let item = map_occurrence(event, occurrence);
        assert_eq!(item.title, "Design Review");
        assert_eq!(item.source, IntegrationSource::Calendar);
        assert!(!item.is_read);
    }

    #[test]
    fn event_uid_falls_back_to_empty_string_when_missing() {
        const NO_UID_ICS: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
SUMMARY:No UID Here\r\n\
DTSTART:20250101T090000Z\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";
        let calendar = parse_first_calendar(NO_UID_ICS).unwrap();
        assert_eq!(event_uid(&calendar.events[0]), "");
    }

    #[tokio::test]
    async fn update_selection_is_not_offered_this_phase() {
        // Documents the deliberate absence, not an oversight: no
        // Picker/SelectionApplier exists for Calendar (step12.md Decision
        // 1's consequence -- no discovery API under the secret-URL model).
        // This test exists so a future reader who adds one notices this
        // comment rather than wondering why it was skipped.
    }
}
