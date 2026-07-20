//! Slack Web API adapter. See `docs/04-extensions/integrations/slack.md`
//! for the endpoint/mapping spec and `step6.md` for the design decisions
//! (polling over Socket Mode, Bot Token over full OAuth, honest-empty over
//! fake demo data, a watch-list over whole-workspace presence).

use crate::polling::{max_option, next_status, retry_after_seconds, to_event_status, PollResult};
use crate::{ConnectionStatus, IntegrationAdapter, IntegrationConnector};
use async_trait::async_trait;
use common::{Result, WorkspaceError};
use domain::{
    IntegrationSource, MemberPresence, NotificationId, NotificationItem, PresenceStatus,
    PriorityLevel, UserId,
};
use events::{Event, EventBus, IntegrationConnectionStatus};
use secrecy::ExposeSecret;
use secrets::{SecretProvider, SecretWriter};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

const SLACK_TOKEN_KEY: &str = "SLACK_BOT_TOKEN";
const SLACK_API_BASE: &str = "https://slack.com/api";

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
    config: Arc<RwLock<SlackConfig>>,
    http: reqwest::Client,
    state: Arc<RwLock<AdapterState>>,
    display_name_cache: Arc<Mutex<HashMap<String, String>>>,
    channel_cursor: Arc<Mutex<HashMap<String, String>>>,
    poll_task: Mutex<Option<JoinHandle<()>>>,
    secret_writer: Arc<dyn SecretWriter>,
    /// Serializes every `run_cycle` call (background-loop iterations and
    /// `sync_now`'s one-off cycles alike) against each other, so a manual
    /// `/sync` landing at the same moment as the interval loop's own tick
    /// can't race it over `channel_cursor` -- both would otherwise be able
    /// to read the same "oldest" cursor before either writes it back,
    /// double-publishing the same messages (`step46.md`).
    poll_lock: Arc<Mutex<()>>,
}

impl SlackAdapter {
    /// Create a new adapter. Call [`IntegrationAdapter::initialize`] before
    /// [`IntegrationAdapter::start`]. `secret_writer` is where
    /// [`IntegrationConnector::connect`] persists a token entered through
    /// the setup UI (`step7.md`) — normally a `SecretProviderChain`.
    #[must_use]
    pub fn new(config: SlackConfig, secret_writer: Arc<dyn SecretWriter>) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            http: reqwest::Client::new(),
            state: Arc::new(RwLock::new(AdapterState {
                status: ConnectionStatus::Disconnected,
                consecutive_failures: 0,
                token: None,
            })),
            display_name_cache: Arc::new(Mutex::new(HashMap::new())),
            channel_cursor: Arc::new(Mutex::new(HashMap::new())),
            poll_task: Mutex::new(None),
            secret_writer,
            poll_lock: Arc::new(Mutex::new(())),
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

        let poller = self.make_poller().await;
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

    async fn sync_now(&self, event_bus: Arc<dyn EventBus>) -> Result<()> {
        let Some(token) = self.state.read().await.token.clone() else {
            return Ok(());
        };
        self.make_poller().await.run_cycle(&event_bus, &token).await;
        Ok(())
    }
}

impl SlackAdapter {
    /// Snapshots everything a poll cycle needs, sharing this adapter's
    /// long-lived state via `Arc`/`Mutex` clones rather than copying it --
    /// used both by `start()` (the background loop) and `sync_now` (a
    /// single out-of-band cycle), so both see and mutate the exact same
    /// cursor/status state.
    async fn make_poller(&self) -> SlackPoller {
        SlackPoller {
            http: self.http.clone(),
            config: self.config.read().await.clone(),
            state: Arc::clone(&self.state),
            display_name_cache: Arc::clone(&self.display_name_cache),
            channel_cursor: Arc::clone(&self.channel_cursor),
            poll_lock: Arc::clone(&self.poll_lock),
        }
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

#[async_trait]
impl IntegrationConnector for SlackAdapter {
    async fn connect(&self, event_bus: Arc<dyn EventBus>, token: String) -> Result<()> {
        self.secret_writer
            .set_secret(SLACK_TOKEN_KEY, &token)
            .await?;
        {
            let mut state = self.state.write().await;
            state.token = Some(token);
            state.status = ConnectionStatus::Connecting;
            state.consecutive_failures = 0;
        }
        // Instant feedback for the setup overlay (step7.md) rather than
        // waiting for the first poll cycle to publish anything.
        let _ = event_bus
            .publish(Event::IntegrationStatusChanged {
                source: IntegrationSource::Slack,
                status: IntegrationConnectionStatus::Connecting,
            })
            .await;
        // Idempotent whether this is the first connection or a reconnect
        // with a replacement token: stop whatever poll loop (if any) is
        // already running before starting a fresh one, so a reconnect
        // never leaves two loops racing against the same shared state.
        self.shutdown().await?;
        self.start(event_bus).await
    }
}

impl SlackAdapter {
    /// Replace the polled channel/watched-user lists (`step8.md`'s picker)
    /// and restart the poll loop with them. Deliberately not part of any
    /// trait — this only touches Integration-context state (the adapter's
    /// own config + poll loop), not `config.toml` persistence, which is a
    /// separate cross-context concern wired at the composition root
    /// (`crates/app/src/main.rs`), not something this crate knows about.
    pub async fn update_selection(
        &self,
        event_bus: Arc<dyn EventBus>,
        channel_ids: Vec<String>,
        watched_user_ids: Vec<String>,
    ) -> Result<()> {
        {
            let mut config = self.config.write().await;
            config.channel_ids = channel_ids;
            config.watched_user_ids = watched_user_ids;
        }
        // Stale cursors for channels no longer selected are dead weight;
        // a newly-added channel has no prior cursor anyway.
        self.channel_cursor.lock().await.clear();
        self.shutdown().await?;
        self.start(event_bus).await
    }
}

/// A Slack channel/user available to pick from, resolved via
/// [`SlackPicker`] (`step8.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerChannel {
    /// Slack channel id (`C...`).
    pub id: String,
    /// Channel name without the leading `#`.
    pub name: String,
}

/// See [`PickerChannel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerUser {
    /// Slack user id (`U...`).
    pub id: String,
    /// Display name (falls back to real name — same rule as message
    /// author resolution).
    pub display_name: String,
}

/// Narrow read-only port listing what's available to select for
/// `SlackConfig.channel_ids`/`watched_user_ids` (`step8.md`). Deliberately
/// not routed through `Command`/`CommandHandler` — listing is a read, and
/// CQRS's own split (commands mutate, queries read) means it doesn't
/// belong on the write path; `crates/ui` holds this port directly.
#[async_trait]
pub trait SlackPicker: Send + Sync {
    /// Public *and* private channels the bot has already been invited to
    /// (`step8.md` Decision 3 — channels it hasn't joined would fail if
    /// selected; `step29.md` — a real bug report: a channel the bot was
    /// just invited to didn't appear here because it was private and
    /// `conversations.list`'s `types` param was hardcoded to
    /// `public_channel` only. Reading a private channel's history also
    /// needs the `groups:history`/`groups:read` bot scopes in addition to
    /// `channels:history`/`channels:read` — see `docs/04-extensions/integrations/slack.md`).
    async fn list_channels(&self) -> Result<Vec<PickerChannel>>;
    /// Non-bot, non-deleted workspace members.
    async fn list_users(&self) -> Result<Vec<PickerUser>>;
}

#[async_trait]
impl SlackPicker for SlackAdapter {
    async fn list_channels(&self) -> Result<Vec<PickerChannel>> {
        let token = self.state.read().await.token.clone().ok_or_else(|| {
            WorkspaceError::Integration("Slack is not configured (no token found)".into())
        })?;
        fetch_channel_list(&self.http, &token).await
    }

    async fn list_users(&self) -> Result<Vec<PickerUser>> {
        let token = self.state.read().await.token.clone().ok_or_else(|| {
            WorkspaceError::Integration("Slack is not configured (no token found)".into())
        })?;
        fetch_user_list(&self.http, &token).await
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
    poll_lock: Arc<Mutex<()>>,
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
            // `oldest.is_none()` means this channel has never been polled
            // in this process's lifetime -- `channel_cursor` isn't
            // persisted across restarts (`step33.md`). `step33.md`
            // originally skipped publishing entirely for this case, but
            // that silently dropped genuinely new messages that happened
            // to already exist at the moment of the very first poll (a
            // real bug, reported via live use: a message sent moments
            // before/during a restart never appeared, and the cursor
            // advanced past it so no later poll ever caught it either).
            // `step39.md` publishes unconditionally instead, so the
            // Notification panel always reflects what's actually unread,
            // and marks the item already-read when it's first-poll
            // backlog so `DesktopNotifier` (`crates/notifications`)
            // knows not to re-toast for something that was already
            // sitting in the channel before this session started.
            let is_first_poll_for_channel = oldest.is_none();
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
                        let mut item = map_message_to_notification(channel_id, msg, &display_name);
                        item.is_read = is_first_poll_for_channel;
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

    /// One poll cycle's worth of work: `poll_once`, the consecutive-failure
    /// state machine, and status-change event publishing -- everything
    /// `run_loop` used to do inline except the trailing sleep, and
    /// everything `sync_now` needs to run exactly once, out-of-band
    /// (`step46.md`). Returns how long the caller should wait before the
    /// next cycle; `sync_now` ignores it, since it isn't scheduling a next
    /// cycle. Serialized against every other `run_cycle` call on this same
    /// adapter via `poll_lock`, so a manual sync landing mid-interval can't
    /// race the background loop's own tick over `channel_cursor`.
    async fn run_cycle(&self, event_bus: &Arc<dyn EventBus>, token: &str) -> Duration {
        let _guard = self.poll_lock.lock().await;
        let base_interval = Duration::from_secs(self.config.sync_interval_secs.max(1));
        let (result, retry_after) = self.poll_once(event_bus, token).await;

        let (status, prev_status) = {
            let mut state = self.state.write().await;
            let prev_status = state.status.clone();
            let (failures, status) = next_status(&prev_status, state.consecutive_failures, result);
            state.consecutive_failures = failures;
            state.status = status.clone();
            (status, prev_status)
        };

        if status != prev_status {
            let _ = event_bus
                .publish(Event::IntegrationStatusChanged {
                    source: IntegrationSource::Slack,
                    status: to_event_status(&status),
                })
                .await;
        }

        if let ConnectionStatus::Failed(reason) = &status {
            if !matches!(prev_status, ConnectionStatus::Failed(_)) {
                let _ = event_bus
                    .publish(Event::SystemAlert(format!(
                        "Slack integration failed: {reason}"
                    )))
                    .await;
            }
        }

        match (result, retry_after) {
            (PollResult::RateLimited, Some(secs)) => Duration::from_secs(secs),
            _ => base_interval,
        }
    }

    async fn run_loop(self, event_bus: Arc<dyn EventBus>, token: String) {
        loop {
            let wait = self.run_cycle(&event_bus, &token).await;
            tokio::time::sleep(wait).await;
        }
    }
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

#[derive(Debug, Deserialize)]
struct RawChannel {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    is_member: bool,
}

#[derive(Debug, Deserialize)]
struct ResponseMetadata {
    #[serde(default)]
    next_cursor: String,
}

#[derive(Debug, Deserialize)]
struct ConversationsListResponse {
    ok: bool,
    #[serde(default)]
    channels: Vec<RawChannel>,
    #[serde(default)]
    response_metadata: Option<ResponseMetadata>,
    #[serde(default)]
    error: Option<String>,
}

/// Pure page-processing step, split out from the network call so pagination
/// (Slack's "empty string means no more pages" convention, easy to get
/// backwards) is unit-testable against fixture JSON without live network —
/// same pattern as the message/presence mapping functions above.
fn extract_channel_page(
    body: ConversationsListResponse,
) -> Result<(Vec<PickerChannel>, Option<String>)> {
    if !body.ok {
        return Err(WorkspaceError::Integration(
            body.error
                .unwrap_or_else(|| "unknown Slack API error".into()),
        ));
    }
    let channels = body
        .channels
        .into_iter()
        .filter(|c| c.is_member)
        .map(|c| PickerChannel {
            id: c.id,
            name: c.name,
        })
        .collect();
    let next_cursor = body
        .response_metadata
        .map(|m| m.next_cursor)
        .filter(|c| !c.is_empty());
    Ok((channels, next_cursor))
}

/// Interactive fetches (the picker's) aren't a background loop with a
/// consecutive-failure counter to absorb a `429` gracefully like the poll
/// loop's `fetch_history`/`fetch_presence` — the user is watching, so a
/// clear "rate limited, try again in Ns" error beats a silent retry or a
/// confusing "response parse failed" (what trying to deserialize a 429
/// body as the expected JSON shape would otherwise produce).
fn rate_limit_error(headers: &reqwest::header::HeaderMap) -> WorkspaceError {
    match retry_after_seconds(headers) {
        Some(secs) => WorkspaceError::Integration(format!(
            "Slack API 속도 제한에 걸렸습니다 — {secs}초 후 다시 시도해주세요."
        )),
        None => WorkspaceError::Integration(
            "Slack API 속도 제한에 걸렸습니다 — 잠시 후 다시 시도해주세요.".into(),
        ),
    }
}

async fn fetch_channel_list(http: &reqwest::Client, token: &str) -> Result<Vec<PickerChannel>> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        // `types` was hardcoded to `public_channel` only, which silently
        // dropped every private channel regardless of `is_member` --
        // real bug report (`step29.md`), a channel the bot was just
        // invited to never showed up here because Slack's
        // `conversations.list` won't return private conversations at all
        // unless `private_channel` is explicitly requested.
        let mut query = vec![
            ("types", "public_channel,private_channel".to_string()),
            ("limit", "200".to_string()),
        ];
        if let Some(c) = &cursor {
            query.push(("cursor", c.clone()));
        }
        let response = http
            .get(format!("{SLACK_API_BASE}/conversations.list"))
            .bearer_auth(token)
            .query(&query)
            .send()
            .await
            .map_err(|e| WorkspaceError::Integration(format!("Slack request failed: {e}")))?;
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(rate_limit_error(response.headers()));
        }
        let body: ConversationsListResponse = response.json().await.map_err(|e| {
            WorkspaceError::Integration(format!("Slack response parse failed: {e}"))
        })?;
        let (mut page, next_cursor) = extract_channel_page(body)?;
        all.append(&mut page);
        match next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(all)
}

#[derive(Debug, Deserialize)]
struct RawUser {
    id: String,
    #[serde(default)]
    is_bot: bool,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    real_name: String,
    #[serde(default)]
    profile: Option<SlackProfile>,
}

#[derive(Debug, Deserialize)]
struct UsersListResponse {
    ok: bool,
    #[serde(default)]
    members: Vec<RawUser>,
    #[serde(default)]
    response_metadata: Option<ResponseMetadata>,
    #[serde(default)]
    error: Option<String>,
}

/// Same role as [`extract_channel_page`], for `users.list`.
fn extract_user_page(body: UsersListResponse) -> Result<(Vec<PickerUser>, Option<String>)> {
    if !body.ok {
        return Err(WorkspaceError::Integration(
            body.error
                .unwrap_or_else(|| "unknown Slack API error".into()),
        ));
    }
    let users = body
        .members
        .into_iter()
        .filter(|u| !u.is_bot && !u.deleted)
        .map(|u| {
            let display_name = u
                .profile
                .map(|p| p.display_name)
                .filter(|n| !n.is_empty())
                .unwrap_or(u.real_name);
            PickerUser {
                id: u.id,
                display_name,
            }
        })
        .collect();
    let next_cursor = body
        .response_metadata
        .map(|m| m.next_cursor)
        .filter(|c| !c.is_empty());
    Ok((users, next_cursor))
}

async fn fetch_user_list(http: &reqwest::Client, token: &str) -> Result<Vec<PickerUser>> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut query = vec![("limit", "200".to_string())];
        if let Some(c) = &cursor {
            query.push(("cursor", c.clone()));
        }
        let response = http
            .get(format!("{SLACK_API_BASE}/users.list"))
            .bearer_auth(token)
            .query(&query)
            .send()
            .await
            .map_err(|e| WorkspaceError::Integration(format!("Slack request failed: {e}")))?;
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(rate_limit_error(response.headers()));
        }
        let body: UsersListResponse = response.json().await.map_err(|e| {
            WorkspaceError::Integration(format!("Slack response parse failed: {e}"))
        })?;
        let (mut page, next_cursor) = extract_user_page(body)?;
        all.append(&mut page);
        match next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(all)
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

    fn test_config() -> SlackConfig {
        SlackConfig {
            channel_ids: vec!["C1".into()],
            watched_user_ids: vec!["U1".into()],
            sync_interval_secs: 30,
        }
    }

    fn test_adapter() -> (SlackAdapter, Arc<RecordingWriter>) {
        let writer = Arc::new(RecordingWriter::default());
        let adapter =
            SlackAdapter::new(test_config(), Arc::clone(&writer) as Arc<dyn SecretWriter>);
        (adapter, writer)
    }

    #[tokio::test]
    async fn initialize_with_no_token_reports_disconnected_not_error() {
        let (adapter, _writer) = test_adapter();
        let result = adapter.initialize(&NoneProvider).await;
        assert!(result.is_ok());
        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Disconnected
        );
    }

    #[tokio::test]
    async fn initialize_with_token_reports_connecting() {
        let (adapter, _writer) = test_adapter();
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
    async fn connect_persists_the_token_and_transitions_to_connecting() {
        let (adapter, writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;

        adapter
            .connect(Arc::clone(&event_bus), "xoxb-from-ui".to_string())
            .await
            .unwrap();

        assert_eq!(
            writer.written.lock().await.as_slice(),
            [(SLACK_TOKEN_KEY.to_string(), "xoxb-from-ui".to_string())]
        );
        // Connecting, not Connected -- the first real poll cycle (which
        // needs live network) hasn't run yet; this only proves the local
        // state transition + persistence, not a real Slack round-trip.
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
                "xoxb-test".to_string(),
            )
            .await
            .unwrap();

        let event = rx
            .try_recv()
            .expect("connect() must publish a status event");
        match event {
            Event::IntegrationStatusChanged { source, status } => {
                assert_eq!(source, IntegrationSource::Slack);
                assert_eq!(status, IntegrationConnectionStatus::Connecting);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn connect_is_safe_to_call_twice_in_a_row() {
        // Reconnecting with a replacement token must not panic or leave
        // two poll loops racing -- shutdown-then-start makes this
        // idempotent regardless of whether a loop was already running.
        let (adapter, _writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;

        adapter
            .connect(Arc::clone(&event_bus), "xoxb-first".to_string())
            .await
            .unwrap();
        adapter
            .connect(Arc::clone(&event_bus), "xoxb-second".to_string())
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
        let (adapter, _writer) = test_adapter();
        adapter.initialize(&NoneProvider).await.unwrap();
        assert!(adapter.start(event_bus).await.is_ok());
    }

    #[tokio::test]
    async fn sync_now_without_credential_is_a_harmless_no_op() {
        // Same "not configured is not an error" reasoning as start()
        // (`step46.md`) -- no token means nothing to poll, not a failure.
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;
        let (adapter, _writer) = test_adapter();
        adapter.initialize(&NoneProvider).await.unwrap();
        assert!(adapter.sync_now(event_bus).await.is_ok());
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
    fn rate_limit_error_includes_the_wait_time_when_known() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "30".parse().unwrap());
        let err = rate_limit_error(&headers);
        assert!(err.to_string().contains("30"));
    }

    #[test]
    fn rate_limit_error_has_a_readable_fallback_when_wait_time_is_unknown() {
        let headers = reqwest::header::HeaderMap::new();
        let err = rate_limit_error(&headers);
        assert!(err.to_string().contains("속도 제한"));
    }

    #[test]
    fn extract_channel_page_filters_to_member_channels_only() {
        let body: ConversationsListResponse = serde_json::from_str(
            r#"{
                "ok": true,
                "channels": [
                    {"id": "C1", "name": "general", "is_member": true},
                    {"id": "C2", "name": "not-joined", "is_member": false}
                ]
            }"#,
        )
        .unwrap();
        let (channels, cursor) = extract_channel_page(body).unwrap();
        assert_eq!(
            channels,
            vec![PickerChannel {
                id: "C1".into(),
                name: "general".into()
            }]
        );
        assert_eq!(cursor, None);
    }

    #[test]
    fn extract_channel_page_reports_the_next_cursor_when_present() {
        let body: ConversationsListResponse = serde_json::from_str(
            r#"{
                "ok": true,
                "channels": [],
                "response_metadata": {"next_cursor": "abc123"}
            }"#,
        )
        .unwrap();
        let (_channels, cursor) = extract_channel_page(body).unwrap();
        assert_eq!(cursor, Some("abc123".to_string()));
    }

    #[test]
    fn extract_channel_page_treats_empty_cursor_string_as_no_more_pages() {
        // Slack's actual "last page" convention: response_metadata is
        // present but next_cursor is "" -- easy to get backwards (treating
        // presence of the field as "more pages" instead of its content).
        let body: ConversationsListResponse = serde_json::from_str(
            r#"{
                "ok": true,
                "channels": [],
                "response_metadata": {"next_cursor": ""}
            }"#,
        )
        .unwrap();
        let (_channels, cursor) = extract_channel_page(body).unwrap();
        assert_eq!(cursor, None);
    }

    #[test]
    fn extract_user_page_filters_out_bots_and_deleted_users() {
        let body: UsersListResponse = serde_json::from_str(
            r#"{
                "ok": true,
                "members": [
                    {"id": "U1", "real_name": "Alice", "is_bot": false, "deleted": false},
                    {"id": "U2", "real_name": "SlackBot", "is_bot": true, "deleted": false},
                    {"id": "U3", "real_name": "Gone", "is_bot": false, "deleted": true}
                ]
            }"#,
        )
        .unwrap();
        let (users, _cursor) = extract_user_page(body).unwrap();
        assert_eq!(
            users,
            vec![PickerUser {
                id: "U1".into(),
                display_name: "Alice".into()
            }]
        );
    }

    #[test]
    fn extract_page_errors_on_slack_api_error() {
        let body: ConversationsListResponse =
            serde_json::from_str(r#"{"ok": false, "channels": [], "error": "invalid_auth"}"#)
                .unwrap();
        let result = extract_channel_page(body);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn update_selection_replaces_the_config_and_restarts_polling() {
        let (adapter, _writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;
        adapter
            .connect(Arc::clone(&event_bus), "xoxb-test".to_string())
            .await
            .unwrap();

        adapter
            .update_selection(
                Arc::clone(&event_bus),
                vec!["C-new".to_string()],
                vec!["U-new".to_string()],
            )
            .await
            .unwrap();

        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Connecting
        );
    }
}
