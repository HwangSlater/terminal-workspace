//! Google Calendar (iCal secret-URL) adapter. See
//! `docs/04-extensions/integrations/calendar.md` for the endpoint/mapping
//! spec, `step12.md` for the original single-calendar design decisions
//! (secret iCal URL over OAuth, a real `RRULE` expansion dependency over
//! hand-rolled parsing), and `step24.md` for multi-calendar support.

use crate::polling::{next_status, retry_after_seconds, to_event_status, PollResult};
use crate::ConnectionStatus;
use crate::{IntegrationAdapter, IntegrationConnector, Picker, PickerItem};
use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
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
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Superseded by `CALENDAR_CONNECTIONS_KEY` (`step24.md`) -- kept only so
/// `initialize()` can migrate a pre-multi-calendar install's single saved
/// URL instead of silently dropping it.
const CALENDAR_URL_KEY: &str = "CALENDAR_ICAL_URL";
const CALENDAR_CONNECTIONS_KEY: &str = "CALENDAR_CONNECTIONS";

/// Fixed namespace for deriving deterministic per-occurrence notification
/// ids from `"{event UID}#{occurrence start epoch millis}"` (UUIDv5) — the
/// same occurrence re-polled upserts instead of duplicating. Distinct from
/// Slack's/GitHub's namespaces so the three integrations can never collide.
const CALENDAR_OCCURRENCE_ID_NAMESPACE: Uuid = Uuid::from_bytes([
    0x9a, 0x1c, 0x2e, 0x3f, 0x4a, 0x5b, 0x46, 0x71, 0x88, 0x92, 0xa3, 0xb4, 0xc5, 0xd6, 0xe7, 0xf8,
]);

/// Static per-integration configuration (non-secret) — the secret iCal
/// URLs themselves come from `SecretProviderChain`, never from this struct.
#[derive(Debug, Clone)]
pub struct CalendarConfig {
    /// Only occurrences starting within this many hours from "now" become
    /// a notification — a reminder feature, not a full calendar dump.
    pub lookahead_hours: u64,
    /// Seconds between poll cycles.
    pub sync_interval_secs: u64,
}

/// One connected calendar (`step24.md`). `id` is a stable identifier
/// independent of `label` -- two calendars can legitimately share a label
/// (nothing stops someone naming two calendars "회의"), but
/// [`Picker`]/`SelectionApplier` need a value that's unique per connection
/// to select against.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CalendarConnection {
    id: Uuid,
    label: String,
    url: String,
}

struct AdapterState {
    status: ConnectionStatus,
    consecutive_failures: u32,
    connections: Vec<CalendarConnection>,
}

/// Google Calendar adapter, driven by one or more calendars' secret iCal
/// feed URLs rather than OAuth (`step12.md` Decision 1). Polls all
/// configured calendars on a shared interval — same rationale as
/// `SlackAdapter`/`GitHubAdapter`. `Picker` here is a local read of
/// already-connected calendars (for removal), not a remote discovery call
/// — the secret-URL model still has no "list my calendars" API, so there's
/// nothing to discover, only what's already been added (`step24.md`).
pub struct CalendarAdapter {
    config: Arc<RwLock<CalendarConfig>>,
    http: reqwest::Client,
    state: Arc<RwLock<AdapterState>>,
    seen_occurrences: Arc<Mutex<HashSet<(Uuid, String, i64)>>>,
    poll_task: Mutex<Option<JoinHandle<()>>>,
    secret_writer: Arc<dyn SecretWriter>,
}

impl CalendarAdapter {
    /// Create a new adapter. Call [`IntegrationAdapter::initialize`] before
    /// [`IntegrationAdapter::start`]. `secret_writer` is where
    /// [`IntegrationConnector::connect`] persists the connection list
    /// entered through the setup UI — normally a `SecretProviderChain`.
    #[must_use]
    pub fn new(config: CalendarConfig, secret_writer: Arc<dyn SecretWriter>) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            http: reqwest::Client::new(),
            state: Arc::new(RwLock::new(AdapterState {
                status: ConnectionStatus::Disconnected,
                consecutive_failures: 0,
                connections: Vec::new(),
            })),
            seen_occurrences: Arc::new(Mutex::new(HashSet::new())),
            poll_task: Mutex::new(None),
            secret_writer,
        }
    }

    async fn persist_connections(&self, connections: &[CalendarConnection]) -> Result<()> {
        let json = serde_json::to_string(connections).map_err(|e| {
            WorkspaceError::Integration(format!("failed to serialize calendar connections: {e}"))
        })?;
        self.secret_writer
            .set_secret(CALENDAR_CONNECTIONS_KEY, &json)
            .await
    }

    /// Removes every connection whose id isn't in `keep_ids` (string-form
    /// UUIDs, per [`PickerItem::id`]), persists the result, and restarts
    /// polling with what's left -- the "apply a picker selection" half of
    /// `step24.md` Decision 3, mirroring `GitHubAdapter::update_selection`.
    pub async fn keep_only(
        &self,
        event_bus: Arc<dyn EventBus>,
        keep_ids: Vec<String>,
    ) -> Result<()> {
        let connections = {
            let mut state = self.state.write().await;
            state
                .connections
                .retain(|c| keep_ids.contains(&c.id.to_string()));
            state.connections.clone()
        };
        self.persist_connections(&connections).await?;
        // A removed calendar leaves dead entries in the seen-set; harmless
        // (a few stale tuples), but clearing avoids unbounded growth across
        // repeated add/remove cycles -- same reasoning
        // `GitHubAdapter::update_selection` already uses.
        self.seen_occurrences.lock().await.clear();
        self.shutdown().await?;
        self.start(event_bus).await
    }
}

#[async_trait]
impl IntegrationAdapter for CalendarAdapter {
    async fn initialize(&self, secret_provider: &dyn SecretProvider) -> Result<()> {
        let connections = match secret_provider.get_secret(CALENDAR_CONNECTIONS_KEY).await? {
            Some(secret) => serde_json::from_str::<Vec<CalendarConnection>>(secret.expose_secret())
                .map_err(|e| {
                    WorkspaceError::Integration(format!(
                        "failed to parse saved calendar connections: {e}"
                    ))
                })?,
            // No multi-calendar list saved yet -- fall back to a
            // pre-step24.md single URL rather than silently losing an
            // existing connection on upgrade (`step24.md` Decision 1).
            // Auto-labeled since no label was ever collected for it.
            None => match secret_provider.get_secret(CALENDAR_URL_KEY).await? {
                Some(secret) => vec![CalendarConnection {
                    id: Uuid::new_v4(),
                    label: "캘린더".to_string(),
                    url: secret.expose_secret().to_string(),
                }],
                None => Vec::new(),
            },
        };

        let mut state = self.state.write().await;
        state.status = if connections.is_empty() {
            ConnectionStatus::Disconnected
        } else {
            ConnectionStatus::Connecting
        };
        state.connections = connections;
        Ok(())
    }

    async fn start(&self, event_bus: Arc<dyn EventBus>) -> Result<()> {
        let connections = self.state.read().await.connections.clone();
        if connections.is_empty() {
            tracing::info!(
                "Calendar adapter has no connections; staying Disconnected (Zero Configuration)."
            );
            return Ok(());
        }

        let poller = CalendarPoller {
            http: self.http.clone(),
            config: self.config.read().await.clone(),
            state: Arc::clone(&self.state),
            seen_occurrences: Arc::clone(&self.seen_occurrences),
        };
        let handle = tokio::spawn(poller.run_loop(event_bus, connections));
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
    /// Adds a calendar rather than replacing the connection set (the
    /// original single-calendar meaning `step12.md` gave this method) --
    /// `step24.md` Decision 2. `token` carries both fields the setup
    /// overlay collects, `"{label}\n{url}"`: reusing the existing
    /// `IntegrationConnector`/`Command::Connect` plumbing needed no new
    /// `Command` variant or `KeyOutcome`, since a label can't itself
    /// contain a newline (the command-bar text capture never inserts one
    /// mid-field -- `Enter` always ends a field instead).
    async fn connect(&self, event_bus: Arc<dyn EventBus>, token: String) -> Result<()> {
        let (label, url) = token.split_once('\n').ok_or_else(|| {
            WorkspaceError::Integration(
                "internal error: calendar connect token missing the label/URL separator".into(),
            )
        })?;
        let label = label.trim().to_string();
        let url = url.trim().to_string();

        // Validate before ever saving or polling -- a non-absolute URL
        // (most commonly: the "https://calendar.google.com/..." prefix got
        // dropped while copying the secret address) previously sailed
        // straight through to `Connecting`, then sat there forever: the
        // poll loop's `reqwest::Client::get` fails with an opaque "relative
        // URL without a base" that was never surfaced anywhere a user could
        // see it before a live-bug follow-up added poll-failure logging.
        // Rejecting it immediately, with a clear reason, means the setup
        // overlay shows `Failed(...)` right away instead of a
        // never-resolving "연결 중...".
        match reqwest::Url::parse(&url) {
            Ok(parsed) if parsed.scheme() == "http" || parsed.scheme() == "https" => {}
            _ => {
                return Err(WorkspaceError::Integration(
                    "Calendar URL must be a full http(s) address (e.g. https://calendar.google.com/calendar/ical/.../basic.ics) -- got a value that isn't a valid absolute URL".into(),
                ));
            }
        }

        let connections = {
            let mut state = self.state.write().await;
            state.connections.push(CalendarConnection {
                id: Uuid::new_v4(),
                label,
                url,
            });
            state.connections.clone()
        };
        self.persist_connections(&connections).await?;
        {
            let mut state = self.state.write().await;
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

#[async_trait]
impl Picker for CalendarAdapter {
    /// The *currently connected* calendars, not a remote discovery list --
    /// there's no "list my calendars" API under the secret-URL auth model
    /// (unchanged since `step12.md`). This is what `Ctrl+K`'s picker
    /// overlay shows to select which calendars to *keep* (`step24.md`
    /// Decision 3), the inverse of GitHub's "select which repos to add."
    async fn list_items(&self) -> Result<Vec<PickerItem>> {
        Ok(self
            .state
            .read()
            .await
            .connections
            .iter()
            .map(|c| PickerItem {
                id: c.id.to_string(),
                label: c.label.clone(),
            })
            .collect())
    }
}

#[async_trait]
impl crate::CalendarManager for CalendarAdapter {
    async fn set_lookahead_hours(&self, event_bus: Arc<dyn EventBus>, hours: u64) -> Result<()> {
        self.config.write().await.lookahead_hours = hours;
        // The running poller snapshotted the old config at `start()` time
        // (`step12.md`'s original shape, unchanged by `step24.md`) -- a
        // restart is the only way a config change reaches it, same
        // approach `keep_only` already uses.
        self.shutdown().await?;
        self.start(event_bus).await
    }

    async fn rename(&self, id: String, new_label: String) -> Result<()> {
        let Ok(target) = id.parse::<Uuid>() else {
            return Err(WorkspaceError::Integration(format!(
                "'{id}' is not a valid calendar id"
            )));
        };
        let connections = {
            let mut state = self.state.write().await;
            let Some(connection) = state.connections.iter_mut().find(|c| c.id == target) else {
                return Err(WorkspaceError::Integration(
                    "no connected calendar with that id".into(),
                ));
            };
            connection.label = new_label;
            state.connections.clone()
        };
        // Cosmetic only -- doesn't affect fetching, so no poll restart
        // needed, just persist the renamed label.
        self.persist_connections(&connections).await
    }

    async fn events_in_range(
        &self,
        after: DateTime<Utc>,
        before: DateTime<Utc>,
    ) -> Result<Vec<NotificationItem>> {
        let connections = self.state.read().await.connections.clone();
        let mut items = Vec::new();
        for connection in &connections {
            let calendar = match fetch_calendar_feed(&self.http, connection).await {
                FetchOutcome::Success(cal) => cal,
                // Best-effort, same "one bad calendar doesn't block the
                // others" spirit as the poll loop (`step24.md` Decision
                // 5) -- a grid view with one calendar's worth of gaps is
                // still useful; an all-or-nothing failure wouldn't be.
                FetchOutcome::RateLimited(_) | FetchOutcome::Failure => continue,
            };
            for event in &calendar.events {
                if let Ok(occurrences) = expand_occurrences(event, after, before) {
                    for occurrence in occurrences {
                        items.push(map_occurrence(event, occurrence, &connection.label));
                    }
                }
            }
        }
        Ok(items)
    }
}

struct CalendarPoller {
    http: reqwest::Client,
    config: CalendarConfig,
    state: Arc<RwLock<AdapterState>>,
    seen_occurrences: Arc<Mutex<HashSet<(Uuid, String, i64)>>>,
}

impl CalendarPoller {
    /// Polls one calendar connection. Failures are logged and reported to
    /// the caller but never propagate as a panic/early-return out of a
    /// whole `poll_once` cycle -- one bad calendar must not stop the others
    /// from being polled (`step24.md` Decision 5).
    async fn poll_one(
        &self,
        event_bus: &Arc<dyn EventBus>,
        connection: &CalendarConnection,
    ) -> (PollResult, Option<u64>) {
        let calendar = match fetch_calendar_feed(&self.http, connection).await {
            FetchOutcome::Success(cal) => cal,
            FetchOutcome::RateLimited(retry_after) => {
                return (PollResult::RateLimited, retry_after)
            }
            FetchOutcome::Failure => return (PollResult::Failure, None),
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
                let key = (
                    connection.id,
                    event_uid(event),
                    occurrence.timestamp_millis(),
                );
                let already_seen = self.seen_occurrences.lock().await.contains(&key);
                if already_seen {
                    continue;
                }
                let item = map_occurrence(event, occurrence, &connection.label);
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

    /// One cycle across every configured calendar. Overall status only
    /// degrades to `Failure` if *every* connection failed this cycle --
    /// one bad calendar must not mask the others working (`step24.md`
    /// Decision 5). `retry_after` is the max of any rate-limited
    /// connection's requested delay.
    async fn poll_once(
        &self,
        event_bus: &Arc<dyn EventBus>,
        connections: &[CalendarConnection],
    ) -> (PollResult, Option<u64>) {
        let mut any_success = false;
        let mut any_rate_limited = false;
        let mut retry_after = None;

        for connection in connections {
            let (result, retry) = self.poll_one(event_bus, connection).await;
            match result {
                PollResult::Success => any_success = true,
                PollResult::RateLimited => {
                    any_rate_limited = true;
                    retry_after = crate::polling::max_option(retry_after, retry);
                }
                PollResult::Failure => {}
            }
        }

        if any_success {
            (PollResult::Success, None)
        } else if any_rate_limited {
            (PollResult::RateLimited, retry_after)
        } else {
            (PollResult::Failure, None)
        }
    }

    async fn run_loop(self, event_bus: Arc<dyn EventBus>, connections: Vec<CalendarConnection>) {
        let base_interval = Duration::from_secs(self.config.sync_interval_secs.max(1));
        loop {
            let (result, retry_after) = self.poll_once(&event_bus, &connections).await;

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

/// Outcome of fetching and parsing one calendar's feed -- shared between
/// [`CalendarPoller::poll_one`] and [`CalendarAdapter::events_in_range`]
/// (`step25.md`), which both need "get me this connection's `VCALENDAR`"
/// but differ in what to do with a rate limit (the poller waits and
/// retries; a one-shot grid-view fetch just gives up on that connection
/// for this request) and dedup/publish (only the poller's job).
enum FetchOutcome {
    Success(ical::parser::ical::component::IcalCalendar),
    RateLimited(Option<u64>),
    Failure,
}

/// `GET`s `connection.url` and parses the returned feed. Every failure
/// path logs the real reason via `tracing::warn!` (visible in the `Ctrl+4`
/// log viewer) before returning -- the same diagnostic discipline a live
/// "stuck on 연결 중..." bug established for the poll loop specifically;
/// this fetch path serves both callers, so both get it.
async fn fetch_calendar_feed(
    http: &reqwest::Client,
    connection: &CalendarConnection,
) -> FetchOutcome {
    let response = match http.get(&connection.url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "Calendar fetch failed for '{}': request error: {e}",
                connection.label
            );
            return FetchOutcome::Failure;
        }
    };

    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return FetchOutcome::RateLimited(retry_after_seconds(response.headers()));
    }
    if !response.status().is_success() {
        // The single most common real-world cause: the secret iCal URL is
        // wrong (a stale/revoked link, or the "public" calendar HTML page
        // URL pasted in by mistake instead of Settings -> "Secret address
        // in iCal format") -- surfacing the actual status code is the
        // difference between a user staring at a silently-stuck "연결
        // 중..." header and being able to self-diagnose via Ctrl+4.
        tracing::warn!(
            "Calendar fetch failed for '{}': HTTP {} from the configured iCal URL",
            connection.label,
            response.status()
        );
        return FetchOutcome::Failure;
    }
    let body = match response.text().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                "Calendar fetch failed for '{}': reading response body: {e}",
                connection.label
            );
            return FetchOutcome::Failure;
        }
    };

    match parse_first_calendar(&body) {
        Ok(cal) => FetchOutcome::Success(cal),
        Err(e) => {
            tracing::warn!(
                "Calendar fetch failed for '{}': could not parse iCal feed: {e}",
                connection.label
            );
            FetchOutcome::Failure
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

/// `label` is prefixed onto the title (`"[{label}] {summary}"`,
/// `step24.md` Decision 4) so multiple calendars stay distinguishable once
/// merged into one Notification/Calendar panel — the whole reason
/// multi-calendar support was worth building this label at all.
fn map_occurrence(
    event: &IcalEvent,
    occurrence: chrono::DateTime<rrule::Tz>,
    label: &str,
) -> NotificationItem {
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
        title: format!("[{label}] {summary}"),
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

    /// Simulates a real upgrade: only the old singular key has a value,
    /// the new list key has none.
    struct LegacySingleUrlProvider(&'static str);

    #[async_trait]
    impl SecretProvider for LegacySingleUrlProvider {
        async fn get_secret(&self, key: &str) -> Result<Option<secrecy::SecretString>> {
            if key == CALENDAR_URL_KEY {
                Ok(Some(secrecy::SecretString::from(self.0.to_string())))
            } else {
                Ok(None)
            }
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

    fn connect_token(label: &str, url: &str) -> String {
        format!("{label}\n{url}")
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
    async fn initialize_with_a_saved_connection_list_reports_connecting() {
        let (adapter, _writer) = test_adapter();
        let saved = serde_json::to_string(&vec![CalendarConnection {
            id: Uuid::new_v4(),
            label: "회사".to_string(),
            url: "https://calendar.google.com/calendar/ical/x/private-y/basic.ics".to_string(),
        }])
        .unwrap();
        adapter
            .initialize(&FixedProvider(Box::leak(saved.into_boxed_str())))
            .await
            .unwrap();
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Connecting
        );
    }

    /// Real backward-compatibility requirement, not a nice-to-have
    /// (`step24.md` Decision 1): a pre-multi-calendar install has a secret
    /// saved under the old singular key, nothing under the new list key.
    /// Upgrading must not silently drop that connection.
    #[tokio::test]
    async fn initialize_migrates_a_legacy_single_url_into_a_one_item_connection_list() {
        let (adapter, _writer) = test_adapter();
        adapter
            .initialize(&LegacySingleUrlProvider(
                "https://calendar.google.com/calendar/ical/x/private-y/basic.ics",
            ))
            .await
            .unwrap();
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Connecting
        );
        let items = adapter.list_items().await.unwrap();
        assert_eq!(items.len(), 1);
    }

    #[tokio::test]
    async fn connect_adds_a_calendar_without_dropping_an_existing_one() {
        let (adapter, _writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;

        adapter
            .connect(
                Arc::clone(&event_bus),
                connect_token("회사", "https://example.com/work.ics"),
            )
            .await
            .unwrap();
        adapter
            .connect(
                Arc::clone(&event_bus),
                connect_token("개인", "https://example.com/personal.ics"),
            )
            .await
            .unwrap();

        let items = adapter.list_items().await.unwrap();
        assert_eq!(items.len(), 2);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"회사"));
        assert!(labels.contains(&"개인"));
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Connecting
        );
    }

    /// Real regression test: a URL missing its `https://` prefix (the most
    /// common real-world mistake -- the scheme dropping off while copying
    /// Google's "secret address in iCal format") must be rejected
    /// immediately, not accepted and left to fail silently on the first
    /// poll cycle with an opaque `reqwest` "relative URL without a base"
    /// error nobody could see.
    #[tokio::test]
    async fn connect_rejects_a_url_missing_its_scheme() {
        let (adapter, writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;

        let result = adapter
            .connect(
                Arc::clone(&event_bus),
                connect_token("회사", "calendar.google.com/calendar/ical/xxx/basic.ics"),
            )
            .await;

        assert!(result.is_err());
        // Must not have been saved -- a bad credential shouldn't survive a
        // restart to keep silently failing.
        assert!(writer.written.lock().await.is_empty());
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Disconnected
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
                connect_token("회사", "https://example.com/cal.ics"),
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
    async fn keep_only_removes_calendars_not_in_the_list_and_persists_the_result() {
        let (adapter, writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;
        adapter
            .connect(
                Arc::clone(&event_bus),
                connect_token("회사", "https://example.com/work.ics"),
            )
            .await
            .unwrap();
        adapter
            .connect(
                Arc::clone(&event_bus),
                connect_token("개인", "https://example.com/personal.ics"),
            )
            .await
            .unwrap();
        let keep_id = adapter.list_items().await.unwrap()[0].id.clone();

        adapter
            .keep_only(Arc::clone(&event_bus), vec![keep_id.clone()])
            .await
            .unwrap();

        let items = adapter.list_items().await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, keep_id);
        // The persisted secret must reflect the removal too, not just
        // in-memory state -- otherwise a restart would resurrect the
        // "removed" calendar.
        let last_write = writer.written.lock().await;
        let (_, saved_json) = last_write.last().unwrap();
        let saved: Vec<CalendarConnection> = serde_json::from_str(saved_json).unwrap();
        assert_eq!(saved.len(), 1);
    }

    #[tokio::test]
    async fn set_lookahead_hours_updates_the_config_and_keeps_polling() {
        use crate::CalendarManager;

        let (adapter, _writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;
        adapter
            .connect(
                Arc::clone(&event_bus),
                connect_token("회사", "https://example.com/work.ics"),
            )
            .await
            .unwrap();

        adapter
            .set_lookahead_hours(Arc::clone(&event_bus), 48)
            .await
            .unwrap();

        assert_eq!(adapter.config.read().await.lookahead_hours, 48);
        // Restarted, not left disconnected -- a config change shouldn't
        // silently drop the connection.
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Connecting
        );
    }

    #[tokio::test]
    async fn rename_updates_the_label_and_persists_it() {
        use crate::CalendarManager;

        let (adapter, writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;
        adapter
            .connect(
                Arc::clone(&event_bus),
                connect_token("오타있는이름", "https://example.com/work.ics"),
            )
            .await
            .unwrap();
        let id = adapter.list_items().await.unwrap()[0].id.clone();

        adapter
            .rename(id.clone(), "회사".to_string())
            .await
            .unwrap();

        let items = adapter.list_items().await.unwrap();
        assert_eq!(items[0].label, "회사");
        let last_write = writer.written.lock().await;
        let (_, saved_json) = last_write.last().unwrap();
        let saved: Vec<CalendarConnection> = serde_json::from_str(saved_json).unwrap();
        assert_eq!(saved[0].label, "회사");
    }

    #[tokio::test]
    async fn rename_with_an_unknown_id_is_a_real_error() {
        use crate::CalendarManager;

        let (adapter, _writer) = test_adapter();
        let result = adapter
            .rename(Uuid::new_v4().to_string(), "안될이름".to_string())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn start_without_any_connection_does_not_spawn_a_poll_loop() {
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
    fn maps_occurrence_to_notification_item_with_the_label_prefixed() {
        let calendar = parse_first_calendar(NON_RECURRING_ICS).unwrap();
        let event = &calendar.events[0];
        let occurrence = chrono::DateTime::parse_from_rfc3339("2025-01-01T09:00:00Z")
            .unwrap()
            .with_timezone(&rrule::Tz::UTC);
        let item = map_occurrence(event, occurrence, "회사");
        assert_eq!(item.title, "[회사] Design Review");
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
}
