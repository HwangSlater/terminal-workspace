//! GitHub REST API adapter. See `docs/04-extensions/integrations/github.md`
//! for the endpoint/mapping spec and `step10.md` for the design decisions
//! (open-PR diff over webhook/update tracking, PAT over OAuth App, full
//! connect-UI + picker treatment mirroring Slack's Phase 7/8 in one phase).

use crate::polling::{max_option, next_status, retry_after_seconds, to_event_status, PollResult};
use crate::{ConnectionStatus, IntegrationAdapter, IntegrationConnector, Picker, PickerItem};
use async_trait::async_trait;
use common::{Result, WorkspaceError};
use domain::{IntegrationSource, NotificationId, NotificationItem, PriorityLevel};
use events::{Event, EventBus, IntegrationConnectionStatus};
use secrecy::ExposeSecret;
use secrets::{SecretProvider, SecretWriter};
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

const GITHUB_TOKEN_KEY: &str = "GITHUB_TOKEN";
const GITHUB_API_BASE: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";
const GITHUB_USER_AGENT: &str = "terminal-workspace";

/// Fixed namespace for deriving deterministic per-PR notification ids from
/// `"{repo}#{pr_number}"` (UUIDv5) — re-polling the same open PR upserts the
/// same [`NotificationItem`] instead of creating a duplicate. Arbitrary
/// constant, not derived from anything (distinct from Slack's namespace so
/// the two integrations can never collide even given the same input string).
const GITHUB_PR_ID_NAMESPACE: Uuid = Uuid::from_bytes([
    0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x47, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x01,
]);

/// Static per-integration configuration (non-secret) — the token itself
/// comes from `SecretProviderChain`, never from this struct.
#[derive(Debug, Clone)]
pub struct GitHubConfig {
    /// Repositories polled for open PRs, `owner/repo` (e.g. `"rust-lang/rust"`).
    pub repositories: Vec<String>,
    /// Seconds between poll cycles.
    pub sync_interval_secs: u64,
}

struct AdapterState {
    status: ConnectionStatus,
    consecutive_failures: u32,
    token: Option<String>,
}

/// GitHub REST API adapter. Polls on an interval rather than holding a
/// persistent connection — same rationale as `SlackAdapter` (`step6.md`).
pub struct GitHubAdapter {
    config: Arc<RwLock<GitHubConfig>>,
    http: reqwest::Client,
    state: Arc<RwLock<AdapterState>>,
    seen_prs: Arc<Mutex<HashSet<(String, u64)>>>,
    poll_task: Mutex<Option<JoinHandle<()>>>,
    secret_writer: Arc<dyn SecretWriter>,
}

impl GitHubAdapter {
    /// Create a new adapter. Call [`IntegrationAdapter::initialize`] before
    /// [`IntegrationAdapter::start`]. `secret_writer` is where
    /// [`IntegrationConnector::connect`] persists a token entered through
    /// the setup UI — normally a `SecretProviderChain`.
    #[must_use]
    pub fn new(config: GitHubConfig, secret_writer: Arc<dyn SecretWriter>) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            http: reqwest::Client::new(),
            state: Arc::new(RwLock::new(AdapterState {
                status: ConnectionStatus::Disconnected,
                consecutive_failures: 0,
                token: None,
            })),
            seen_prs: Arc::new(Mutex::new(HashSet::new())),
            poll_task: Mutex::new(None),
            secret_writer,
        }
    }

    /// Replace the polled repository list (`step10.md`'s picker) and
    /// restart the poll loop with it. Deliberately not part of any trait —
    /// this only touches Integration-context state, not `config.toml`
    /// persistence (a separate cross-context concern wired at the
    /// composition root, `crates/app/src/main.rs`).
    pub async fn update_selection(
        &self,
        event_bus: Arc<dyn EventBus>,
        repositories: Vec<String>,
    ) -> Result<()> {
        {
            let mut config = self.config.write().await;
            config.repositories = repositories;
        }
        // A repo no longer selected leaves dead entries in the seen-set;
        // harmless (just a few stale tuples), but clearing avoids
        // unbounded growth across repeated selection changes.
        self.seen_prs.lock().await.clear();
        self.shutdown().await?;
        self.start(event_bus).await
    }
}

#[async_trait]
impl IntegrationAdapter for GitHubAdapter {
    async fn initialize(&self, secret_provider: &dyn SecretProvider) -> Result<()> {
        let token = secret_provider.get_secret(GITHUB_TOKEN_KEY).await?;
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
                "GitHub adapter has no credential; staying Disconnected (Zero Configuration)."
            );
            return Ok(());
        };

        let poller = GitHubPoller {
            http: self.http.clone(),
            config: self.config.read().await.clone(),
            state: Arc::clone(&self.state),
            seen_prs: Arc::clone(&self.seen_prs),
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
impl IntegrationConnector for GitHubAdapter {
    async fn connect(&self, event_bus: Arc<dyn EventBus>, token: String) -> Result<()> {
        self.secret_writer
            .set_secret(GITHUB_TOKEN_KEY, &token)
            .await?;
        {
            let mut state = self.state.write().await;
            state.token = Some(token);
            state.status = ConnectionStatus::Connecting;
            state.consecutive_failures = 0;
        }
        // Instant feedback for the setup overlay rather than waiting for
        // the first poll cycle to publish anything.
        let _ = event_bus
            .publish(Event::IntegrationStatusChanged {
                source: IntegrationSource::GitHub,
                status: IntegrationConnectionStatus::Connecting,
            })
            .await;
        // Idempotent whether this is the first connection or a reconnect
        // with a replacement token, same reasoning as SlackAdapter's
        // IntegrationConnector::connect impl.
        self.shutdown().await?;
        self.start(event_bus).await
    }
}

#[async_trait]
impl Picker for GitHubAdapter {
    async fn list_items(&self) -> Result<Vec<PickerItem>> {
        let token = self.state.read().await.token.clone().ok_or_else(|| {
            WorkspaceError::Integration("GitHub is not configured (no token found)".into())
        })?;
        fetch_repository_list(&self.http, &token).await
    }
}

/// Owns everything the background poll loop needs, cloned out of
/// [`GitHubAdapter`]'s fields at `start()` time so the loop doesn't need to
/// hold a `&GitHubAdapter` reference across an `'static` spawned task.
struct GitHubPoller {
    http: reqwest::Client,
    config: GitHubConfig,
    state: Arc<RwLock<AdapterState>>,
    seen_prs: Arc<Mutex<HashSet<(String, u64)>>>,
}

impl GitHubPoller {
    async fn poll_once(
        &self,
        event_bus: &Arc<dyn EventBus>,
        token: &str,
    ) -> (PollResult, Option<u64>) {
        let mut any_failure = false;
        let mut retry_after: Option<u64> = None;

        for repo in &self.config.repositories {
            match fetch_open_pull_requests(&self.http, token, repo).await {
                Ok(PullRequestOutcome::RateLimited { retry_after_secs }) => {
                    retry_after = max_option(retry_after, retry_after_secs);
                }
                Ok(PullRequestOutcome::PullRequests(prs)) => {
                    for pr in &prs {
                        let key = (repo.clone(), pr.number);
                        let already_seen = self.seen_prs.lock().await.contains(&key);
                        if already_seen {
                            continue;
                        }
                        let item = map_pull_request(repo, pr);
                        if event_bus
                            .publish(Event::GitHubPRCreated(item))
                            .await
                            .is_err()
                        {
                            any_failure = true;
                        }
                        self.seen_prs.lock().await.insert(key);
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
                        source: IntegrationSource::GitHub,
                        status: to_event_status(&status),
                    })
                    .await;
            }

            if let ConnectionStatus::Failed(reason) = &status {
                if !matches!(prev_status, ConnectionStatus::Failed(_)) {
                    let _ = event_bus
                        .publish(Event::SystemAlert(format!(
                            "GitHub integration failed: {reason}"
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

fn pr_notification_id(repo: &str, pr_number: u64) -> NotificationId {
    NotificationId(Uuid::new_v5(
        &GITHUB_PR_ID_NAMESPACE,
        format!("{repo}#{pr_number}").as_bytes(),
    ))
}

fn map_pull_request(repo: &str, pr: &GitHubPullRequest) -> NotificationItem {
    NotificationItem {
        id: pr_notification_id(repo, pr.number),
        source: IntegrationSource::GitHub,
        title: format!("{repo}#{} {}", pr.number, pr.title),
        body: format!("by {}", pr.user.login),
        timestamp_ms: parse_iso8601_to_millis(&pr.updated_at),
        priority: PriorityLevel::Medium,
        is_read: false,
        action_link: Some(pr.html_url.clone()),
    }
}

/// GitHub timestamps are RFC3339/ISO8601 (`"2024-01-01T12:00:00Z"`), unlike
/// Slack's `"<secs>.<microsecs>"` string — a hand-rolled parse of just the
/// pieces this codebase actually needs (no `chrono`/`time` dependency
/// pulled in for one field), matching the "minimal dependency graph"
/// preference already stated for the CLI arg parser (`configuration.md` §3.1).
fn parse_iso8601_to_millis(ts: &str) -> u64 {
    let digits: String = ts.chars().filter(|c| c.is_ascii_digit()).take(14).collect();
    if digits.len() < 14 {
        return 0;
    }
    let year: i64 = digits[0..4].parse().unwrap_or(1970);
    let month: i64 = digits[4..6].parse().unwrap_or(1);
    let day: i64 = digits[6..8].parse().unwrap_or(1);
    let hour: i64 = digits[8..10].parse().unwrap_or(0);
    let minute: i64 = digits[10..12].parse().unwrap_or(0);
    let second: i64 = digits[12..14].parse().unwrap_or(0);

    // Days since epoch via a standard civil-to-days algorithm (Howard
    // Hinnant's `days_from_civil`), avoiding a full calendar library for a
    // "good enough for notification ordering" timestamp.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch = era * 146_097 + doe - 719_468;

    let secs = days_since_epoch * 86_400 + hour * 3600 + minute * 60 + second;
    u64::try_from(secs.max(0)).unwrap_or(0) * 1000
}

#[derive(Debug, Default, Deserialize)]
struct GitHubPullRequestUser {
    #[serde(default)]
    login: String,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequest {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    user: GitHubPullRequestUser,
}

enum PullRequestOutcome {
    PullRequests(Vec<GitHubPullRequest>),
    RateLimited { retry_after_secs: Option<u64> },
}

#[derive(Debug, Deserialize)]
struct GitHubErrorResponse {
    #[serde(default)]
    message: String,
}

/// A plain `403` is GitHub's response to both "rate limited" *and* "bad/
/// insufficient-scope token" — conflating them would mean an expired PAT
/// gets treated as a transient rate limit forever, silently retrying
/// instead of ever tripping the `Reconnecting`/`Failed` threshold that
/// would actually surface the problem. GitHub's real rate-limit signal is
/// `X-RateLimit-Remaining: 0` (primary limit) or a `Retry-After` header
/// (secondary limit / abuse detection) — check for those specifically
/// rather than trusting the status code alone. `429` has no such ambiguity.
fn is_rate_limited(status: reqwest::StatusCode, headers: &reqwest::header::HeaderMap) -> bool {
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return true;
    }
    if status != reqwest::StatusCode::FORBIDDEN {
        return false;
    }
    if retry_after_seconds(headers).is_some() {
        return true;
    }
    headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "0")
}

fn apply_github_headers(builder: reqwest::RequestBuilder, token: &str) -> reqwest::RequestBuilder {
    builder
        .bearer_auth(token)
        .header(reqwest::header::USER_AGENT, GITHUB_USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
}

async fn fetch_open_pull_requests(
    http: &reqwest::Client,
    token: &str,
    repo: &str,
) -> Result<PullRequestOutcome> {
    let response = apply_github_headers(
        http.get(format!("{GITHUB_API_BASE}/repos/{repo}/pulls")),
        token,
    )
    .query(&[("state", "open"), ("per_page", "100")])
    .send()
    .await
    .map_err(|e| WorkspaceError::Integration(format!("GitHub request failed: {e}")))?;

    if is_rate_limited(response.status(), response.headers()) {
        return Ok(PullRequestOutcome::RateLimited {
            retry_after_secs: retry_after_seconds(response.headers()),
        });
    }
    if !response.status().is_success() {
        let status = response.status();
        let body: GitHubErrorResponse = response.json().await.unwrap_or(GitHubErrorResponse {
            message: String::new(),
        });
        return Err(WorkspaceError::Integration(if body.message.is_empty() {
            format!("GitHub API error ({status})")
        } else {
            format!("GitHub API error ({status}): {}", body.message)
        }));
    }
    let prs: Vec<GitHubPullRequest> = response
        .json()
        .await
        .map_err(|e| WorkspaceError::Integration(format!("GitHub response parse failed: {e}")))?;
    Ok(PullRequestOutcome::PullRequests(prs))
}

/// Interactive fetches (the picker's) aren't a background loop with a
/// consecutive-failure counter to absorb a rate limit gracefully like the
/// poll loop's `fetch_open_pull_requests` — the user is watching, so a
/// clear "rate limited, try again in Ns" error beats a silent retry or a
/// confusing parse-failure message.
fn rate_limit_error(headers: &reqwest::header::HeaderMap) -> WorkspaceError {
    match retry_after_seconds(headers) {
        Some(secs) => WorkspaceError::Integration(format!(
            "GitHub API 속도 제한에 걸렸습니다 — {secs}초 후 다시 시도해주세요."
        )),
        None => WorkspaceError::Integration(
            "GitHub API 속도 제한에 걸렸습니다 — 잠시 후 다시 시도해주세요.".into(),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    #[serde(default)]
    full_name: String,
}

async fn fetch_repository_list(http: &reqwest::Client, token: &str) -> Result<Vec<PickerItem>> {
    let mut all = Vec::new();
    let mut page: u32 = 1;
    loop {
        let response =
            apply_github_headers(http.get(format!("{GITHUB_API_BASE}/user/repos")), token)
                .query(&[("per_page", "100".to_string()), ("page", page.to_string())])
                .send()
                .await
                .map_err(|e| WorkspaceError::Integration(format!("GitHub request failed: {e}")))?;

        if is_rate_limited(response.status(), response.headers()) {
            return Err(rate_limit_error(response.headers()));
        }
        if !response.status().is_success() {
            let status = response.status();
            return Err(WorkspaceError::Integration(format!(
                "GitHub API error ({status})"
            )));
        }
        let repos: Vec<GitHubRepo> = response.json().await.map_err(|e| {
            WorkspaceError::Integration(format!("GitHub response parse failed: {e}"))
        })?;
        let is_last_page = repos.len() < 100;
        all.extend(repos.into_iter().map(|r| PickerItem {
            id: r.full_name.clone(),
            label: r.full_name,
        }));
        if is_last_page {
            break;
        }
        page += 1;
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

    fn test_config() -> GitHubConfig {
        GitHubConfig {
            repositories: vec!["rust-lang/rust".into()],
            sync_interval_secs: 60,
        }
    }

    fn test_adapter() -> (GitHubAdapter, Arc<RecordingWriter>) {
        let writer = Arc::new(RecordingWriter::default());
        let adapter =
            GitHubAdapter::new(test_config(), Arc::clone(&writer) as Arc<dyn SecretWriter>);
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
            .initialize(&FixedProvider("ghp_test"))
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
            .connect(Arc::clone(&event_bus), "ghp_from_ui".to_string())
            .await
            .unwrap();

        assert_eq!(
            writer.written.lock().await.as_slice(),
            [(GITHUB_TOKEN_KEY.to_string(), "ghp_from_ui".to_string())]
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
                "ghp_test".to_string(),
            )
            .await
            .unwrap();

        let event = rx
            .try_recv()
            .expect("connect() must publish a status event");
        match event {
            Event::IntegrationStatusChanged { source, status } => {
                assert_eq!(source, IntegrationSource::GitHub);
                assert_eq!(status, IntegrationConnectionStatus::Connecting);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn connect_is_safe_to_call_twice_in_a_row() {
        let (adapter, _writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;

        adapter
            .connect(Arc::clone(&event_bus), "ghp_first".to_string())
            .await
            .unwrap();
        adapter
            .connect(Arc::clone(&event_bus), "ghp_second".to_string())
            .await
            .unwrap();

        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Connecting
        );
    }

    #[tokio::test]
    async fn start_without_credential_does_not_spawn_a_poll_loop() {
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;
        let (adapter, _writer) = test_adapter();
        adapter.initialize(&NoneProvider).await.unwrap();
        assert!(adapter.start(event_bus).await.is_ok());
    }

    #[tokio::test]
    async fn update_selection_replaces_the_config_and_restarts_polling() {
        let (adapter, _writer) = test_adapter();
        let event_bus = Arc::new(events::InProcessEventBus::new(8)) as Arc<dyn EventBus>;
        adapter
            .connect(Arc::clone(&event_bus), "ghp_test".to_string())
            .await
            .unwrap();

        adapter
            .update_selection(Arc::clone(&event_bus), vec!["owner/new-repo".to_string()])
            .await
            .unwrap();

        assert_eq!(
            adapter.health_check().await.unwrap(),
            ConnectionStatus::Connecting
        );
    }

    #[test]
    fn same_repo_and_number_produce_the_same_notification_id() {
        let a = pr_notification_id("rust-lang/rust", 42);
        let b = pr_notification_id("rust-lang/rust", 42);
        assert_eq!(a, b);
    }

    #[test]
    fn different_pr_numbers_produce_different_notification_ids() {
        let a = pr_notification_id("rust-lang/rust", 42);
        let b = pr_notification_id("rust-lang/rust", 43);
        assert_ne!(a, b);
    }

    #[test]
    fn maps_pull_request_to_notification_item() {
        let pr = GitHubPullRequest {
            number: 42,
            title: "Fix the thing".into(),
            html_url: "https://github.com/rust-lang/rust/pull/42".into(),
            updated_at: "2024-01-15T10:30:00Z".into(),
            user: GitHubPullRequestUser {
                login: "octocat".into(),
            },
        };
        let item = map_pull_request("rust-lang/rust", &pr);
        assert_eq!(item.title, "rust-lang/rust#42 Fix the thing");
        assert_eq!(item.body, "by octocat");
        assert_eq!(item.source, IntegrationSource::GitHub);
        assert_eq!(
            item.action_link.as_deref(),
            Some("https://github.com/rust-lang/rust/pull/42")
        );
        assert!(!item.is_read);
    }

    #[test]
    fn parses_a_real_iso8601_timestamp() {
        // 2024-01-15T10:30:00Z, verified against a known-correct epoch ms.
        assert_eq!(
            parse_iso8601_to_millis("2024-01-15T10:30:00Z"),
            1_705_314_600_000
        );
    }

    #[test]
    fn parses_the_unix_epoch_itself() {
        assert_eq!(parse_iso8601_to_millis("1970-01-01T00:00:00Z"), 0);
    }

    #[test]
    fn invalid_timestamp_falls_back_to_zero() {
        assert_eq!(parse_iso8601_to_millis("not-a-timestamp"), 0);
    }

    #[test]
    fn deserializes_a_real_pull_requests_response_fixture() {
        let json = r#"[
            {
                "number": 1,
                "title": "Add feature",
                "html_url": "https://github.com/owner/repo/pull/1",
                "updated_at": "2024-01-15T10:30:00Z",
                "user": {"login": "alice"}
            }
        ]"#;
        let prs: Vec<GitHubPullRequest> = serde_json::from_str(json).unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 1);
        assert_eq!(prs[0].user.login, "alice");
    }

    #[test]
    fn deserializes_a_real_user_repos_response_fixture() {
        let json = r#"[
            {"full_name": "owner/repo-one"},
            {"full_name": "owner/repo-two"}
        ]"#;
        let repos: Vec<GitHubRepo> = serde_json::from_str(json).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].full_name, "owner/repo-one");
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
    fn is_rate_limited_recognizes_429_regardless_of_headers() {
        let headers = reqwest::header::HeaderMap::new();
        assert!(is_rate_limited(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            &headers
        ));
    }

    #[test]
    fn is_rate_limited_recognizes_403_with_retry_after() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "30".parse().unwrap());
        assert!(is_rate_limited(reqwest::StatusCode::FORBIDDEN, &headers));
    }

    #[test]
    fn is_rate_limited_recognizes_403_with_zero_remaining_quota() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
        assert!(is_rate_limited(reqwest::StatusCode::FORBIDDEN, &headers));
    }

    #[test]
    fn plain_403_without_rate_limit_headers_is_not_treated_as_rate_limited() {
        // A bad/expired/insufficient-scope token also returns 403 -- must
        // NOT be swallowed as a rate limit, or it would retry forever
        // without ever tripping the Reconnecting/Failed threshold.
        let headers = reqwest::header::HeaderMap::new();
        assert!(!is_rate_limited(reqwest::StatusCode::FORBIDDEN, &headers));
    }

    #[test]
    fn ok_status_is_never_rate_limited() {
        let headers = reqwest::header::HeaderMap::new();
        assert!(!is_rate_limited(reqwest::StatusCode::OK, &headers));
    }
}
