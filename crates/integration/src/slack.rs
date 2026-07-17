//! Slack Web API adapter. See `docs/04-extensions/integrations/slack.md`
//! for the endpoint/mapping spec and `step6.md` for the design decisions
//! (polling over Socket Mode, Bot Token over full OAuth, honest-empty over
//! fake demo data, a watch-list over whole-workspace presence).

use crate::{ConnectionStatus, IntegrationAdapter};
use async_trait::async_trait;
use common::{Result, WorkspaceError};
use domain::{
    IntegrationSource, MemberPresence, NotificationId, NotificationItem, PresenceStatus,
    PriorityLevel, UserId,
};
use events::{Event, EventBus};
use secrecy::ExposeSecret;
use secrets::SecretProvider;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

const SLACK_TOKEN_KEY: &str = "SLACK_BOT_TOKEN";
const SLACK_API_BASE: &str = "https://slack.com/api";
const RECONNECTING_THRESHOLD: u32 = 5;
const FAILED_THRESHOLD: u32 = 10;

/// Fixed namespace for deriving deterministic per-message notification ids
/// from `channel_id:ts` (UUIDv5) — re-polling the same Slack message
/// upserts the same [`NotificationItem`] instead of creating a duplicate.
/// Arbitrary constant, not derived from anything.
const SLACK_MESSAGE_ID_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6f, 0x6a, 0x9e, 0x60, 0x2c, 0x1e, 0x4f, 0x9a, 0xae, 0x1b, 0x3f, 0x2d, 0x7c, 0x88, 0x11, 0x00,
]);

/// Static per-integration configuration (non-secret) — the token itself
/// comes from `SecretProviderChain`, never from this struct.
#[derive(Debug, Clone)]
pub struct SlackConfig {
    /// Channels polled for new messages (`conversations.history`).
    pub channel_ids: Vec<String>,
    /// Teammates polled for presence (`users.getPresence`) — a configured
    /// watch-list, not the whole workspace roster (see
    /// `docs/04-extensions/integrations/slack.md`).
    pub watched_user_ids: Vec<String>,
    /// Seconds between poll cycles.
    pub sync_interval_secs: u64,
}

/// Narrow outbound port for sending a Slack message, kept separate from the
/// generic [`IntegrationAdapter`] lifecycle trait so `crates/commands` can
/// depend on just this one capability (`Command::SendSlackMessage`)
/// without pulling in adapter lifecycle concerns it has no business
/// managing.
#[async_trait]
pub trait SlackMessenger: Send + Sync {
    /// Post `text` to `channel_id` via `chat.postMessage`.
    async fn send_message(&self, channel_id: &str, text: &str) -> Result<()>;
}

struct AdapterState {
    status: ConnectionStatus,
    consecutive_failures: u32,
    token: Option<String>,
}

/// Slack Web API adapter. Polls on an interval rather than holding a
/// persistent connection — see `step6.md` for why.
pub struct SlackAdapter {
    config: SlackConfig,
    http: reqwest::Client,
    state: Arc<RwLock<AdapterState>>,
    display_name_cache: Arc<Mutex<HashMap<String, String>>>,
    channel_cursor: Arc<Mutex<HashMap<String, String>>>,
    poll_task: Mutex<Option<JoinHandle<()>>>,
}

impl SlackAdapter {
    /// Create a new adapter. Call [`IntegrationAdapter::initialize`] before
    /// [`IntegrationAdapter::start`].
    #[must_use]
    pub fn new(config: SlackConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            state: Arc::new(RwLock::new(AdapterState {
                status: ConnectionStatus::Disconnected,
                consecutive_failures: 0,
                token: None,
            })),
            display_name_cache: Arc::new(Mutex::new(HashMap::new())),
            channel_cursor: Arc::new(Mutex::new(HashMap::new())),
            poll_task: Mutex::new(None),
        }
    }
}

#[async_trait]
impl IntegrationAdapter for SlackAdapter {
    async fn initialize(&self, secret_provider: &dyn SecretProvider) -> Result<()> {
        let token = secret_provider.get_secret(SLACK_TOKEN_KEY).await?;
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
        let token = self.state.read().await.token.clone();
        let Some(token) = token else {
            tracing::info!(
                "Slack adapter has no credential; staying Disconnected (Zero Configuration)."
            );
            return Ok(());
        };

        let poller = SlackPoller {
            http: self.http.clone(),
            config: self.config.clone(),
            state: Arc::clone(&self.state),
            display_name_cache: Arc::clone(&self.display_name_cache),
            channel_cursor: Arc::clone(&self.channel_cursor),
        };
        let handle = tokio::spawn(poller.run_loop(event_bus, token));
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
impl SlackMessenger for SlackAdapter {
    async fn send_message(&self, channel_id: &str, text: &str) -> Result<()> {
        let token = self.state.read().await.token.clone().ok_or_else(|| {
            WorkspaceError::Integration("Slack is not configured (no token found)".into())
        })?;
        post_message(&self.http, &token, channel_id, text).await
    }
}

/// Owns everything the background poll loop needs, cloned out of
/// [`SlackAdapter`]'s fields at `start()` time so the loop doesn't need to
/// hold a `&SlackAdapter` reference across an `'static` spawned task.
struct SlackPoller {
    http: reqwest::Client,
    config: SlackConfig,
    state: Arc<RwLock<AdapterState>>,
    display_name_cache: Arc<Mutex<HashMap<String, String>>>,
    channel_cursor: Arc<Mutex<HashMap<String, String>>>,
}

impl SlackPoller {
    async fn resolve_display_name(&self, token: &str, user_id: &str) -> String {
        if let Some(name) = self.display_name_cache.lock().await.get(user_id) {
            return name.clone();
        }
        // Any failure here (network, rate limit, unknown user) falls back
        // to the raw id rather than affecting the cycle's overall
        // success/failure accounting -- a missing display name is a
        // degraded, not failed, outcome for the item it's attached to.
        let name = fetch_user_display_name(&self.http, token, user_id)
            .await
            .unwrap_or_else(|_| user_id.to_string());
        self.display_name_cache
            .lock()
            .await
            .insert(user_id.to_string(), name.clone());
        name
    }

    async fn poll_once(
        &self,
        event_bus: &Arc<dyn EventBus>,
        token: &str,
    ) -> (PollResult, Option<u64>) {
        let mut any_failure = false;
        let mut retry_after: Option<u64> = None;

        for channel_id in &self.config.channel_ids {
            let oldest = self.channel_cursor.lock().await.get(channel_id).cloned();
            match fetch_history(&self.http, token, channel_id, oldest.as_deref()).await {
                Ok(HistoryOutcome::RateLimited { retry_after_secs }) => {
                    retry_after = max_option(retry_after, retry_after_secs);
                }
                Ok(HistoryOutcome::Messages(messages)) => {
                    let mut latest_ts = oldest;
                    for msg in &messages {
                        let display_name = match &msg.user {
                            Some(uid) => self.resolve_display_name(token, uid).await,
                            None => "Slack".to_string(),
                        };
                        let item = map_message_to_notification(channel_id, msg, &display_name);
                        if event_bus
                            .publish(Event::SlackMessageReceived(item))
                            .await
                            .is_err()
                        {
                            any_failure = true;
                        }
                        if latest_ts.as_deref().is_none_or(|cur| msg.ts.as_str() > cur) {
                            latest_ts = Some(msg.ts.clone());
                        }
                    }
                    if let Some(ts) = latest_ts {
                        self.channel_cursor
                            .lock()
                            .await
                            .insert(channel_id.clone(), ts);
                    }
                }
                Err(_) => any_failure = true,
            }
        }

        for user_id in &self.config.watched_user_ids {
            let display_name = self.resolve_display_name(token, user_id).await;
            match fetch_presence(&self.http, token, user_id).await {
                Ok(PresenceOutcome::RateLimited { retry_after_secs }) => {
                    retry_after = max_option(retry_after, retry_after_secs);
                }
                Ok(PresenceOutcome::Presence(presence)) => {
                    let member =
                        map_presence_to_member(user_id, &display_name, presence.as_deref());
                    if event_bus
                        .publish(Event::SlackPresenceChanged(member))
                        .await
                        .is_err()
                    {
                        any_failure = true;
                    }
                }
                Err(_) => any_failure = true,
            }
        }

        let result = if any_failure {
            PollResult::Failure
        } else if retry_after.is_some() {
            PollResult::RateLimited
        } else {
            PollResult::Success
        };
        (result, retry_after)
    }

    async fn run_loop(self, event_bus: Arc<dyn EventBus>, token: String) {
        let base_interval = Duration::from_secs(self.config.sync_interval_secs.max(1));
        loop {
            let (result, retry_after) = self.poll_once(&event_bus, &token).await;

            let (status, prev_was_failed) = {
                let mut state = self.state.write().await;
                let prev_status = state.status.clone();
                let (failures, status) =
                    next_status(&prev_status, state.consecutive_failures, result);
                state.consecutive_failures = failures;
                state.status = status.clone();
                (status, matches!(prev_status, ConnectionStatus::Failed(_)))
            };

            if let ConnectionStatus::Failed(reason) = &status {
                if !prev_was_failed {
                    let _ = event_bus
                        .publish(Event::SystemAlert(format!(
                            "Slack integration failed: {reason}"
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollResult {
    Success,
    RateLimited,
    Failure,
}

fn max_option(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

/// Pure state transition per `docs/04-extensions/state-machine.md`:
/// a success always resets to `Connected`; a rate-limited cycle is a no-op
/// (not counted as a failure); a failure only changes status once the
/// consecutive-failure count crosses the `Reconnecting`/`Failed` thresholds.
fn next_status(
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

fn retry_after_seconds(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn parse_ts_to_millis(ts: &str) -> u64 {
    ts.parse::<f64>()
        .ok()
        .map(|secs| (secs * 1000.0).round() as u64)
        .unwrap_or(0)
}

fn message_notification_id(channel_id: &str, ts: &str) -> NotificationId {
    NotificationId(Uuid::new_v5(
        &SLACK_MESSAGE_ID_NAMESPACE,
        format!("{channel_id}:{ts}").as_bytes(),
    ))
}

fn map_message_to_notification(
    channel_id: &str,
    msg: &SlackMessage,
    display_name: &str,
) -> NotificationItem {
    NotificationItem {
        id: message_notification_id(channel_id, &msg.ts),
        source: IntegrationSource::Slack,
        title: display_name.to_string(),
        body: msg.text.clone(),
        timestamp_ms: parse_ts_to_millis(&msg.ts),
        priority: PriorityLevel::Medium,
        is_read: false,
        action_link: None,
    }
}

fn map_presence_to_member(
    user_id: &str,
    display_name: &str,
    presence: Option<&str>,
) -> MemberPresence {
    let status = match presence {
        Some("active") => PresenceStatus::Active,
        Some(_) => PresenceStatus::Away,
        None => PresenceStatus::Offline,
    };
    MemberPresence {
        user_id: UserId(user_id.to_string()),
        display_name: display_name.to_string(),
        status,
        custom_status_text: None,
        last_updated_ms: now_ms(),
    }
}

#[derive(Debug, Deserialize)]
struct SlackMessage {
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    text: String,
    ts: String,
}

#[derive(Debug, Deserialize)]
struct HistoryResponse {
    ok: bool,
    #[serde(default)]
    messages: Vec<SlackMessage>,
    #[serde(default)]
    error: Option<String>,
}

enum HistoryOutcome {
    Messages(Vec<SlackMessage>),
    RateLimited { retry_after_secs: Option<u64> },
}

async fn fetch_history(
    http: &reqwest::Client,
    token: &str,
    channel_id: &str,
    oldest: Option<&str>,
) -> Result<HistoryOutcome> {
    let mut query = vec![("channel", channel_id.to_string())];
    if let Some(oldest) = oldest {
        query.push(("oldest", oldest.to_string()));
    }
    let response = http
        .get(format!("{SLACK_API_BASE}/conversations.history"))
        .bearer_auth(token)
        .query(&query)
        .send()
        .await
        .map_err(|e| WorkspaceError::Integration(format!("Slack request failed: {e}")))?;

    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Ok(HistoryOutcome::RateLimited {
            retry_after_secs: retry_after_seconds(response.headers()),
        });
    }
    let body: HistoryResponse = response
        .json()
        .await
        .map_err(|e| WorkspaceError::Integration(format!("Slack response parse failed: {e}")))?;
    if !body.ok {
        return Err(WorkspaceError::Integration(
            body.error
                .unwrap_or_else(|| "unknown Slack API error".into()),
        ));
    }
    Ok(HistoryOutcome::Messages(body.messages))
}

#[derive(Debug, Deserialize)]
struct PresenceResponse {
    ok: bool,
    #[serde(default)]
    presence: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

enum PresenceOutcome {
    Presence(Option<String>),
    RateLimited { retry_after_secs: Option<u64> },
}

async fn fetch_presence(
    http: &reqwest::Client,
    token: &str,
    user_id: &str,
) -> Result<PresenceOutcome> {
    let response = http
        .get(format!("{SLACK_API_BASE}/users.getPresence"))
        .bearer_auth(token)
        .query(&[("user", user_id)])
        .send()
        .await
        .map_err(|e| WorkspaceError::Integration(format!("Slack request failed: {e}")))?;

    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Ok(PresenceOutcome::RateLimited {
            retry_after_secs: retry_after_seconds(response.headers()),
        });
    }
    let body: PresenceResponse = response
        .json()
        .await
        .map_err(|e| WorkspaceError::Integration(format!("Slack response parse failed: {e}")))?;
    if !body.ok {
        return Err(WorkspaceError::Integration(
            body.error
                .unwrap_or_else(|| "unknown Slack API error".into()),
        ));
    }
    Ok(PresenceOutcome::Presence(body.presence))
}

#[derive(Debug, Deserialize)]
struct UserInfoResponse {
    ok: bool,
    #[serde(default)]
    user: Option<SlackUser>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackUser {
    #[serde(default)]
    real_name: String,
    #[serde(default)]
    profile: Option<SlackProfile>,
}

#[derive(Debug, Deserialize)]
struct SlackProfile {
    #[serde(default)]
    display_name: String,
}

async fn fetch_user_display_name(
    http: &reqwest::Client,
    token: &str,
    user_id: &str,
) -> Result<String> {
    let response = http
        .get(format!("{SLACK_API_BASE}/users.info"))
        .bearer_auth(token)
        .query(&[("user", user_id)])
        .send()
        .await
        .map_err(|e| WorkspaceError::Integration(format!("Slack request failed: {e}")))?;
    let body: UserInfoResponse = response
        .json()
        .await
        .map_err(|e| WorkspaceError::Integration(format!("Slack response parse failed: {e}")))?;
    if !body.ok {
        return Err(WorkspaceError::Integration(
            body.error
                .unwrap_or_else(|| "unknown Slack API error".into()),
        ));
    }
    let user = body
        .user
        .ok_or_else(|| WorkspaceError::Integration("Slack user not found".into()))?;
    let name = user
        .profile
        .map(|p| p.display_name)
        .filter(|n| !n.is_empty())
        .unwrap_or(user.real_name);
    Ok(name)
}

#[derive(Debug, Deserialize)]
struct PostMessageResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

async fn post_message(
    http: &reqwest::Client,
    token: &str,
    channel_id: &str,
    text: &str,
) -> Result<()> {
    let response = http
        .post(format!("{SLACK_API_BASE}/chat.postMessage"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "channel": channel_id, "text": text }))
        .send()
        .await
        .map_err(|e| WorkspaceError::Integration(format!("Slack request failed: {e}")))?;
    let body: PostMessageResponse = response
        .json()
        .await
        .map_err(|e| WorkspaceError::Integration(format!("Slack response parse failed: {e}")))?;
    if !body.ok {
        return Err(WorkspaceError::Integration(
            body.error
                .unwrap_or_else(|| "unknown Slack API error".into()),
        ));
    }
    Ok(())
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

    fn test_config() -> SlackConfig {
        SlackConfig {
            channel_ids: vec!["C1".into()],
            watched_user_ids: vec!["U1".into()],
            sync_interval_secs: 30,
        }
    }

    #[tokio::test]
    async fn initialize_with_no_token_reports_disconnected_not_error() {
        let adapter = SlackAdapter::new(test_config());
        let result = adapter.initialize(&NoneProvider).await;
        assert!(result.is_ok());
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Disconnected
        );
    }

    #[tokio::test]
    async fn initialize_with_token_reports_connecting() {
        let adapter = SlackAdapter::new(test_config());
        adapter
            .initialize(&FixedProvider("xoxb-test"))
            .await
            .unwrap();
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Connecting
        );
    }

    #[tokio::test]
    async fn start_without_credential_does_not_spawn_a_poll_loop() {
        // No direct way to observe "no task spawned" other than that this
        // returns quickly and without error, matching integration-contract.md
        // §2.3 ("must not return an error or abort").
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;
        let adapter = SlackAdapter::new(test_config());
        adapter.initialize(&NoneProvider).await.unwrap();
        assert!(adapter.start(event_bus).await.is_ok());
    }

    #[test]
    fn parses_slack_timestamp_to_milliseconds() {
        assert_eq!(parse_ts_to_millis("1699999999.000100"), 1_699_999_999_000);
    }

    #[test]
    fn invalid_timestamp_falls_back_to_zero() {
        assert_eq!(parse_ts_to_millis("not-a-timestamp"), 0);
    }

    #[test]
    fn same_channel_and_ts_produce_the_same_notification_id() {
        let a = message_notification_id("C1", "1699999999.000100");
        let b = message_notification_id("C1", "1699999999.000100");
        assert_eq!(a, b);
    }

    #[test]
    fn different_ts_produce_different_notification_ids() {
        let a = message_notification_id("C1", "1699999999.000100");
        let b = message_notification_id("C1", "1699999999.000200");
        assert_ne!(a, b);
    }

    #[test]
    fn maps_slack_message_to_notification_item() {
        let msg = SlackMessage {
            user: Some("U1".into()),
            text: "hello team".into(),
            ts: "1699999999.000100".into(),
        };
        let item = map_message_to_notification("C1", &msg, "Alice");
        assert_eq!(item.title, "Alice");
        assert_eq!(item.body, "hello team");
        assert_eq!(item.source, IntegrationSource::Slack);
        assert!(!item.is_read);
    }

    #[test]
    fn maps_active_presence() {
        let member = map_presence_to_member("U1", "Alice", Some("active"));
        assert_eq!(member.status, PresenceStatus::Active);
    }

    #[test]
    fn maps_away_presence() {
        let member = map_presence_to_member("U1", "Alice", Some("away"));
        assert_eq!(member.status, PresenceStatus::Away);
    }

    #[test]
    fn maps_missing_presence_to_offline() {
        let member = map_presence_to_member("U1", "Alice", None);
        assert_eq!(member.status, PresenceStatus::Offline);
    }

    #[test]
    fn deserializes_a_real_history_response_fixture() {
        let json = r#"{
            "ok": true,
            "messages": [
                {"type": "message", "user": "U1", "text": "hi", "ts": "1699999999.000100"}
            ]
        }"#;
        let body: HistoryResponse = serde_json::from_str(json).unwrap();
        assert!(body.ok);
        assert_eq!(body.messages.len(), 1);
        assert_eq!(body.messages[0].text, "hi");
    }

    #[test]
    fn deserializes_a_real_presence_response_fixture() {
        let json = r#"{"ok": true, "presence": "active"}"#;
        let body: PresenceResponse = serde_json::from_str(json).unwrap();
        assert!(body.ok);
        assert_eq!(body.presence.as_deref(), Some("active"));
    }

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
