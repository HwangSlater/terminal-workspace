//! CQRS Command definitions and Command Handler traits.

use async_trait::async_trait;
use common::{Result, WorkspaceError};
use domain::{
    IntegrationSource, MemberPresence, NotificationId, NotificationItem, NotificationRepository,
    PresenceRepository, PresenceStatus, UserId,
};
use events::{Event, EventBus, EventHandler};
use integration::{IntegrationConnector, SlackMessenger};
use logging::{spans::command_span, TraceContext};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::Instrument;

/// Strongly-typed CQRS system write commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Mutate developer status.
    SetPresence {
        /// Presence target enum.
        status: PresenceStatus,
        /// Custom message status.
        custom_text: Option<String>,
    },

    /// Dispatch message through Slack integration.
    SendSlackMessage {
        /// Channel target.
        channel_id: String,
        /// Text message payload.
        text: String,
    },

    /// Persist a credential (entered through an integration's setup
    /// overlay — Slack's `step7.md`, GitHub's `step10.md`) and connect with
    /// it. Generalized in `step11.md` from separate `ConnectSlack`/
    /// `ConnectGitHub` variants once both turned out to have byte-for-byte
    /// identical shape. `token`'s raw value never appears in logs even in a
    /// `{:?}` dump of this command — the logging pipeline's secret-scrubbing
    /// writer (`crates/logging/src/redact.rs`) masks any `xoxb-`/`ghp_`-
    /// prefixed substring regardless of where it appears.
    Connect {
        /// Which integration to connect.
        source: IntegrationSource,
        /// Bot Token (Slack, `xoxb-...`) or Personal Access Token (GitHub,
        /// `ghp_...`), depending on `source`.
        token: String,
    },

    /// Persist a channel/watched-user selection made through the Slack
    /// picker overlay (`step8.md`, `Ctrl+P`) to `config.toml` and restart
    /// the Slack poll loop with it. Slack has **two** independent
    /// selectable lists (channels for messages, people for presence) —
    /// this does not generalize into `ApplySelection` below, which is for
    /// integrations with exactly one list (`step11.md`).
    ApplySlackSelection {
        /// Channels to poll for messages (`conversations.history`).
        channel_ids: Vec<String>,
        /// Teammates to poll for presence (`users.getPresence`).
        watched_user_ids: Vec<String>,
    },

    /// Persist a single-list selection made through a picker overlay
    /// (GitHub's `step10.md`, `Ctrl+R`) to `config.toml` and restart that
    /// integration's poll loop with it. Generalized in `step11.md` from
    /// `ApplyGitHubSelection`; only fits integrations with exactly one
    /// selectable list — Slack's two-list selection stays its own variant
    /// above.
    ApplySelection {
        /// Which integration this selection applies to.
        source: IntegrationSource,
        /// Selected item ids (e.g. GitHub `owner/repo` strings).
        items: Vec<String>,
    },

    /// Mark notification as read.
    MarkNotificationRead {
        /// Unique identifier.
        id: NotificationId,
    },

    /// Force synchronization check on all active integration adapters.
    SyncAllAdapters,
}

/// Narrow port for `Command::ApplySlackSelection`: persist a channel/user
/// selection somewhere durable and apply it live. Defined here rather than
/// alongside `SlackConnector`/`SlackMessenger` in `crates/integration`
/// because applying a selection is inherently cross-context — it touches
/// both `config.toml` (Config/Workspace concern) and the adapter's live
/// poll loop (Integration concern) — so `crates/integration` itself has no
/// business knowing about `config.toml`. The concrete implementation is
/// wired at the composition root (`crates/app/src/main.rs`), which is
/// where both halves are already available.
#[async_trait]
pub trait SlackSelectionApplier: Send + Sync {
    /// Persist `channel_ids`/`watched_user_ids` and restart Slack polling
    /// with them.
    async fn apply(
        &self,
        event_bus: Arc<dyn EventBus>,
        channel_ids: Vec<String>,
        watched_user_ids: Vec<String>,
    ) -> Result<()>;
}

/// Narrow port for `Command::ApplySelection` — any integration with a
/// single selectable list (GitHub's repositories, a future Calendar's
/// calendar ids, ...). Same cross-context reasoning as
/// [`SlackSelectionApplier`] (`config.toml` + the live adapter), same
/// composition-root wiring. Generalized in `step11.md` from
/// `GitHubSelectionApplier`, which had this exact shape already.
#[async_trait]
pub trait SelectionApplier: Send + Sync {
    /// Persist `items` and restart that integration's polling with them.
    async fn apply(&self, event_bus: Arc<dyn EventBus>, items: Vec<String>) -> Result<()>;
}

/// Abstract Command Handler executing state mutations.
#[async_trait]
pub trait CommandHandler<C>: Send + Sync {
    /// Result payload.
    type Output;

    /// Execute command mutation.
    async fn handle(&self, command: C) -> Result<Self::Output>;
}

/// Dynamic Command Dispatcher distributing commands to handlers.
#[async_trait]
pub trait CommandDispatcher: Send + Sync {
    /// Route command to registered handler.
    async fn dispatch(&self, command: Command) -> Result<()>;
}

/// Placeholder identity for the single local operator. No auth/identity
/// system exists yet (`docs/01-product/product-requirements.md` doesn't plan one before
/// v1.0.0) — revisit once multi-user or authenticated scenarios are in scope.
const LOCAL_USER_ID: &str = "local-user";

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::SetPresence { .. } => "SetPresence",
        Command::SendSlackMessage { .. } => "SendSlackMessage",
        Command::Connect { .. } => "Connect",
        Command::ApplySlackSelection { .. } => "ApplySlackSelection",
        Command::ApplySelection { .. } => "ApplySelection",
        Command::MarkNotificationRead { .. } => "MarkNotificationRead",
        Command::SyncAllAdapters => "SyncAllAdapters",
    }
}

/// Concrete `CommandHandler<Command>` wiring the write path to storage
/// repositories and the event bus. See `docs/06-development/decisions/0007-cqrs.md`'s
/// Phase 3 amendment.
pub struct WorkspaceCommandHandler {
    presence_repo: Arc<dyn PresenceRepository>,
    notification_repo: Arc<dyn NotificationRepository>,
    event_bus: Arc<dyn EventBus>,
    /// `None` when no Slack adapter was constructed at all — `SendSlackMessage`
    /// then returns the same honest "not available" error it always has,
    /// instead of a fake success (`docs/04-extensions/integration-contract.md` §2.3).
    slack_messenger: Option<Arc<dyn SlackMessenger>>,
    /// `None` when no Slack adapter was constructed at all, same as
    /// `slack_messenger` above — but unlike it, populated even when
    /// `enabled = false` in config, since the picker overlay (`step8.md`)
    /// needs an adapter to apply a selection to even before the user has
    /// configured anything (see `crates/app/src/main.rs`).
    slack_selection_applier: Option<Arc<dyn SlackSelectionApplier>>,
    /// Every integration's `Command::Connect` target, keyed by source.
    /// Absent key = that integration's adapter was never constructed at
    /// all (`step11.md` — replaces the earlier `slack_connector`/
    /// `github_connector` named `Option` fields, which had identical
    /// shape).
    connectors: HashMap<IntegrationSource, Arc<dyn IntegrationConnector>>,
    /// Every single-selectable-list integration's `Command::ApplySelection`
    /// target, keyed by source. Slack's two-list selection is not here —
    /// see `slack_selection_applier` above (`step11.md`).
    selection_appliers: HashMap<IntegrationSource, Arc<dyn SelectionApplier>>,
}

/// Looks up `source` in a connector/applier registry, or returns the same
/// "not available" error shape every integration used to hand-write
/// individually (`step11.md`).
fn require<T: ?Sized>(
    map: &HashMap<IntegrationSource, Arc<T>>,
    source: IntegrationSource,
) -> Result<&Arc<T>> {
    map.get(&source)
        .ok_or_else(|| WorkspaceError::Integration(format!("{source:?} adapter not available")))
}

impl WorkspaceCommandHandler {
    /// Construct a handler wired to the given repositories and event bus.
    /// `slack_messenger`/`slack_selection_applier` are `None` when no Slack
    /// adapter was constructed at all; `connectors`/`selection_appliers`
    /// simply omit a key for any integration that wasn't constructed.
    #[must_use]
    pub fn new(
        presence_repo: Arc<dyn PresenceRepository>,
        notification_repo: Arc<dyn NotificationRepository>,
        event_bus: Arc<dyn EventBus>,
        slack_messenger: Option<Arc<dyn SlackMessenger>>,
        slack_selection_applier: Option<Arc<dyn SlackSelectionApplier>>,
        connectors: HashMap<IntegrationSource, Arc<dyn IntegrationConnector>>,
        selection_appliers: HashMap<IntegrationSource, Arc<dyn SelectionApplier>>,
    ) -> Self {
        Self {
            presence_repo,
            notification_repo,
            event_bus,
            slack_messenger,
            slack_selection_applier,
            connectors,
            selection_appliers,
        }
    }
}

#[async_trait]
impl CommandHandler<Command> for WorkspaceCommandHandler {
    type Output = ();

    async fn handle(&self, command: Command) -> Result<()> {
        match command {
            Command::SetPresence {
                status,
                custom_text,
            } => {
                let presence = MemberPresence {
                    user_id: UserId(LOCAL_USER_ID.to_string()),
                    display_name: LOCAL_USER_ID.to_string(),
                    status,
                    custom_status_text: custom_text,
                    last_updated_ms: now_ms(),
                };
                self.presence_repo.save_presence(&presence).await?;
                // Event::SlackPresenceChanged is the only Event variant (frozen
                // by Architecture Freeze v1) carrying MemberPresence; reused
                // here for locally-originated presence changes. See
                // docs/06-development/decisions/0003-event-bus.md's Phase 3 amendment.
                self.event_bus
                    .publish(Event::SlackPresenceChanged(presence))
                    .await?;
                Ok(())
            }
            Command::MarkNotificationRead { id } => {
                // No Event published: no frozen Event variant fits "notification
                // read," and there's no Projector subscriber yet to receive one.
                self.notification_repo.mark_read(&id).await
            }
            Command::SendSlackMessage { channel_id, text } => match &self.slack_messenger {
                Some(messenger) => messenger.send_message(&channel_id, &text).await,
                None => Err(WorkspaceError::Integration(
                    "Slack integration not configured".into(),
                )),
            },
            Command::Connect { source, token } => {
                let connector = require(&self.connectors, source)?;
                connector.connect(Arc::clone(&self.event_bus), token).await
            }
            Command::ApplySlackSelection {
                channel_ids,
                watched_user_ids,
            } => match &self.slack_selection_applier {
                Some(applier) => {
                    applier
                        .apply(Arc::clone(&self.event_bus), channel_ids, watched_user_ids)
                        .await
                }
                None => Err(WorkspaceError::Integration(
                    "Slack adapter not available".into(),
                )),
            },
            Command::ApplySelection { source, items } => {
                let applier = require(&self.selection_appliers, source)?;
                applier.apply(Arc::clone(&self.event_bus), items).await
            }
            Command::SyncAllAdapters => {
                tracing::info!("SyncAllAdapters requested; no integration adapters registered yet");
                Ok(())
            }
        }
    }
}

/// In-memory `CommandDispatcher` delegating every `Command` to a single
/// injected `WorkspaceCommandHandler`, wrapped in a `command_span` (see
/// `docs/05-operations/logging.md` §0) for the standardized span hierarchy.
pub struct InMemoryCommandDispatcher {
    handler: Arc<WorkspaceCommandHandler>,
}

impl InMemoryCommandDispatcher {
    /// Wrap `handler` as the dispatch target for every `Command`.
    #[must_use]
    pub fn new(handler: Arc<WorkspaceCommandHandler>) -> Self {
        Self { handler }
    }
}

#[async_trait]
impl CommandDispatcher for InMemoryCommandDispatcher {
    async fn dispatch(&self, command: Command) -> Result<()> {
        let ctx = TraceContext::new();
        let span = command_span(command_name(&command), &ctx);
        let handler = Arc::clone(&self.handler);

        async move { handler.handle(command).await }
            .instrument(span)
            .await
    }
}

/// Shared, lock-guarded handle to a [`DashboardReadModel`]. `crates/ui`
/// reads through this directly on every render — never touches storage on
/// a render tick (ADR-0007).
pub type SharedReadModel = Arc<RwLock<DashboardReadModel>>;

/// In-memory CQRS read model powering the TUI. Populated once from storage
/// at startup by [`Projector::new`], then kept current by dispatched
/// `Event`s — never re-queries storage after that.
#[derive(Debug, Clone, Default)]
pub struct DashboardReadModel {
    /// Unread notifications, newest first.
    pub unread_notifications: Vec<NotificationItem>,
    /// All known team members' presence.
    pub team_presence: Vec<MemberPresence>,
}

/// Subscribes to the `EventBus` (via `EventDispatcher::register_handler`,
/// like any other `EventHandler`) and keeps a [`DashboardReadModel`]
/// current. Closes the read path `docs/06-development/decisions/0007-cqrs.md`
/// deferred until a real UI consumer existed.
pub struct Projector {
    model: SharedReadModel,
}

impl Projector {
    /// Build a projector and its initial read model, populated once from
    /// storage. Returns the projector (to register as an `EventHandler`)
    /// and a cloneable handle to the model (for the TUI to read from).
    pub async fn new(
        presence_repo: &Arc<dyn PresenceRepository>,
        notification_repo: &Arc<dyn NotificationRepository>,
    ) -> Result<(Self, SharedReadModel)> {
        let team_presence = presence_repo.fetch_all().await?;
        let unread_notifications = notification_repo.fetch_unread().await?;
        let model = Arc::new(RwLock::new(DashboardReadModel {
            unread_notifications,
            team_presence,
        }));
        Ok((
            Self {
                model: Arc::clone(&model),
            },
            model,
        ))
    }
}

#[async_trait]
impl EventHandler for Projector {
    async fn handle(&self, event: Event) -> Result<()> {
        match event {
            Event::SlackPresenceChanged(presence) => {
                let mut model = self.model.write().await;
                match model
                    .team_presence
                    .iter_mut()
                    .find(|p| p.user_id == presence.user_id)
                {
                    Some(existing) => *existing = presence,
                    None => model.team_presence.push(presence),
                }
            }
            Event::SlackMessageReceived(item)
            | Event::GitHubPRCreated(item)
            | Event::CalendarReminderTriggered(item) => {
                let mut model = self.model.write().await;
                match model
                    .unread_notifications
                    .iter_mut()
                    .find(|n| n.id == item.id)
                {
                    Some(existing) => *existing = item,
                    None => model.unread_notifications.push(item),
                }
                model
                    .unread_notifications
                    .sort_by_key(|n| std::cmp::Reverse(n.timestamp_ms));
            }
            Event::SystemAlert(_) | Event::PluginCustomEvent { .. } => {
                // Not surfaced in the read model yet — no panel renders
                // system alerts or plugin events in Phase 5's scope.
            }
            Event::IntegrationStatusChanged { .. } => {
                // Not a DashboardReadModel concern — crates/ui subscribes
                // to the EventBus directly for this one (step9.md, ADR-0016)
                // so the header can update without waiting for a redraw
                // trigger the Projector would otherwise gate on.
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::NotificationItem;
    use events::InProcessEventBus;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    #[derive(Default)]
    struct MockPresenceRepo {
        saved: Mutex<Vec<MemberPresence>>,
    }

    #[async_trait]
    impl PresenceRepository for MockPresenceRepo {
        async fn save_presence(&self, presence: &MemberPresence) -> Result<()> {
            self.saved.lock().await.push(presence.clone());
            Ok(())
        }

        async fn fetch_all(&self) -> Result<Vec<MemberPresence>> {
            Ok(self.saved.lock().await.clone())
        }
    }

    #[derive(Default)]
    struct MockNotificationRepo {
        saved: Mutex<Vec<NotificationItem>>,
        marked_read: Mutex<Vec<NotificationId>>,
    }

    #[async_trait]
    impl NotificationRepository for MockNotificationRepo {
        async fn save(&self, item: &NotificationItem) -> Result<()> {
            self.saved.lock().await.push(item.clone());
            Ok(())
        }

        async fn find_by_id(&self, _id: &NotificationId) -> Result<Option<NotificationItem>> {
            Ok(None)
        }

        async fn fetch_unread(&self) -> Result<Vec<NotificationItem>> {
            Ok(self.saved.lock().await.clone())
        }

        async fn mark_read(&self, id: &NotificationId) -> Result<()> {
            self.marked_read.lock().await.push(id.clone());
            Ok(())
        }
    }

    type Fixture = (
        Arc<WorkspaceCommandHandler>,
        Arc<MockPresenceRepo>,
        Arc<MockNotificationRepo>,
        Arc<InProcessEventBus>,
    );

    fn make_handler() -> Fixture {
        make_handler_with(None, None, HashMap::new(), HashMap::new())
    }

    fn make_handler_with_slack(
        slack_messenger: Option<Arc<dyn SlackMessenger>>,
        slack_connector: Option<Arc<dyn IntegrationConnector>>,
        slack_selection_applier: Option<Arc<dyn SlackSelectionApplier>>,
    ) -> Fixture {
        let mut connectors = HashMap::new();
        if let Some(c) = slack_connector {
            connectors.insert(IntegrationSource::Slack, c);
        }
        make_handler_with(
            slack_messenger,
            slack_selection_applier,
            connectors,
            HashMap::new(),
        )
    }

    fn make_handler_with(
        slack_messenger: Option<Arc<dyn SlackMessenger>>,
        slack_selection_applier: Option<Arc<dyn SlackSelectionApplier>>,
        connectors: HashMap<IntegrationSource, Arc<dyn IntegrationConnector>>,
        selection_appliers: HashMap<IntegrationSource, Arc<dyn SelectionApplier>>,
    ) -> Fixture {
        let presence = Arc::new(MockPresenceRepo::default());
        let notifications = Arc::new(MockNotificationRepo::default());
        let bus = Arc::new(InProcessEventBus::new(10));
        let handler = Arc::new(WorkspaceCommandHandler::new(
            Arc::clone(&presence) as Arc<dyn PresenceRepository>,
            Arc::clone(&notifications) as Arc<dyn NotificationRepository>,
            Arc::clone(&bus) as Arc<dyn EventBus>,
            slack_messenger,
            slack_selection_applier,
            connectors,
            selection_appliers,
        ));
        (handler, presence, notifications, bus)
    }

    #[tokio::test]
    async fn set_presence_persists_and_publishes_event() {
        let (handler, presence, _notifications, bus) = make_handler();
        let mut rx = bus.subscribe();

        handler
            .handle(Command::SetPresence {
                status: PresenceStatus::Away,
                custom_text: Some("brb".into()),
            })
            .await
            .unwrap();

        assert_eq!(presence.saved.lock().await.len(), 1);
        assert_eq!(presence.saved.lock().await[0].status, PresenceStatus::Away);

        let event = rx.try_recv().expect("event should have been published");
        match event {
            Event::SlackPresenceChanged(p) => {
                assert_eq!(p.custom_status_text.as_deref(), Some("brb"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn mark_notification_read_persists_no_event() {
        let (handler, _presence, notifications, bus) = make_handler();
        let mut rx = bus.subscribe();
        let id = NotificationId(Uuid::new_v4());

        handler
            .handle(Command::MarkNotificationRead { id: id.clone() })
            .await
            .unwrap();

        assert_eq!(notifications.marked_read.lock().await.as_slice(), [id]);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn send_slack_message_errors_without_integration() {
        let (handler, ..) = make_handler();
        let result = handler
            .handle(Command::SendSlackMessage {
                channel_id: "C1".into(),
                text: "hi".into(),
            })
            .await;
        assert!(result.is_err());
    }

    struct MockSlackMessenger {
        sent: Mutex<Vec<(String, String)>>,
        should_fail: bool,
    }

    #[async_trait]
    impl SlackMessenger for MockSlackMessenger {
        async fn send_message(&self, channel_id: &str, text: &str) -> Result<()> {
            if self.should_fail {
                return Err(common::WorkspaceError::Integration("boom".into()));
            }
            self.sent
                .lock()
                .await
                .push((channel_id.to_string(), text.to_string()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn send_slack_message_delegates_to_the_configured_messenger() {
        let messenger = Arc::new(MockSlackMessenger {
            sent: Mutex::new(Vec::new()),
            should_fail: false,
        });
        let (handler, ..) = make_handler_with_slack(
            Some(Arc::clone(&messenger) as Arc<dyn SlackMessenger>),
            None,
            None,
        );

        handler
            .handle(Command::SendSlackMessage {
                channel_id: "C1".into(),
                text: "hi".into(),
            })
            .await
            .unwrap();

        assert_eq!(
            messenger.sent.lock().await.as_slice(),
            [("C1".to_string(), "hi".to_string())]
        );
    }

    #[tokio::test]
    async fn send_slack_message_propagates_messenger_errors() {
        let messenger = Arc::new(MockSlackMessenger {
            sent: Mutex::new(Vec::new()),
            should_fail: true,
        });
        let (handler, ..) = make_handler_with_slack(Some(messenger), None, None);

        let result = handler
            .handle(Command::SendSlackMessage {
                channel_id: "C1".into(),
                text: "hi".into(),
            })
            .await;
        assert!(result.is_err());
    }

    struct MockConnector {
        connected: Mutex<Vec<String>>,
        should_fail: bool,
    }

    #[async_trait]
    impl IntegrationConnector for MockConnector {
        async fn connect(&self, _event_bus: Arc<dyn EventBus>, token: String) -> Result<()> {
            if self.should_fail {
                return Err(common::WorkspaceError::Integration("boom".into()));
            }
            self.connected.lock().await.push(token);
            Ok(())
        }
    }

    #[tokio::test]
    async fn connect_errors_without_a_matching_adapter() {
        let (handler, ..) = make_handler();
        let result = handler
            .handle(Command::Connect {
                source: IntegrationSource::Slack,
                token: "xoxb-test".into(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_delegates_to_the_connector_registered_for_that_source() {
        let connector = Arc::new(MockConnector {
            connected: Mutex::new(Vec::new()),
            should_fail: false,
        });
        let (handler, ..) = make_handler_with_slack(
            None,
            Some(Arc::clone(&connector) as Arc<dyn IntegrationConnector>),
            None,
        );

        handler
            .handle(Command::Connect {
                source: IntegrationSource::Slack,
                token: "xoxb-from-ui".into(),
            })
            .await
            .unwrap();

        assert_eq!(
            connector.connected.lock().await.as_slice(),
            ["xoxb-from-ui".to_string()]
        );
    }

    #[tokio::test]
    async fn connect_propagates_connector_errors() {
        let connector = Arc::new(MockConnector {
            connected: Mutex::new(Vec::new()),
            should_fail: true,
        });
        let (handler, ..) = make_handler_with_slack(
            None,
            Some(Arc::clone(&connector) as Arc<dyn IntegrationConnector>),
            None,
        );

        let result = handler
            .handle(Command::Connect {
                source: IntegrationSource::Slack,
                token: "xoxb-test".into(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_does_not_dispatch_to_an_unregistered_source_even_if_another_is_registered() {
        // Two integrations registered under different sources must not
        // cross-wire -- a connector registered for Slack must not answer
        // for GitHub.
        let connector = Arc::new(MockConnector {
            connected: Mutex::new(Vec::new()),
            should_fail: false,
        });
        let (handler, ..) = make_handler_with_slack(
            None,
            Some(Arc::clone(&connector) as Arc<dyn IntegrationConnector>),
            None,
        );

        let result = handler
            .handle(Command::Connect {
                source: IntegrationSource::GitHub,
                token: "ghp_test".into(),
            })
            .await;
        assert!(result.is_err());
        assert!(connector.connected.lock().await.is_empty());
    }

    struct MockSlackSelectionApplier {
        applied: Mutex<Vec<(Vec<String>, Vec<String>)>>,
        should_fail: bool,
    }

    #[async_trait]
    impl SlackSelectionApplier for MockSlackSelectionApplier {
        async fn apply(
            &self,
            _event_bus: Arc<dyn EventBus>,
            channel_ids: Vec<String>,
            watched_user_ids: Vec<String>,
        ) -> Result<()> {
            if self.should_fail {
                return Err(common::WorkspaceError::Integration("boom".into()));
            }
            self.applied
                .lock()
                .await
                .push((channel_ids, watched_user_ids));
            Ok(())
        }
    }

    #[tokio::test]
    async fn apply_slack_selection_errors_without_an_adapter() {
        let (handler, ..) = make_handler();
        let result = handler
            .handle(Command::ApplySlackSelection {
                channel_ids: vec!["C1".into()],
                watched_user_ids: vec!["U1".into()],
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn apply_slack_selection_delegates_to_the_configured_applier() {
        let applier = Arc::new(MockSlackSelectionApplier {
            applied: Mutex::new(Vec::new()),
            should_fail: false,
        });
        let (handler, ..) = make_handler_with_slack(
            None,
            None,
            Some(Arc::clone(&applier) as Arc<dyn SlackSelectionApplier>),
        );

        handler
            .handle(Command::ApplySlackSelection {
                channel_ids: vec!["C1".into()],
                watched_user_ids: vec!["U1".into()],
            })
            .await
            .unwrap();

        assert_eq!(
            applier.applied.lock().await.as_slice(),
            [(vec!["C1".to_string()], vec!["U1".to_string()])]
        );
    }

    #[tokio::test]
    async fn apply_slack_selection_propagates_applier_errors() {
        let applier = Arc::new(MockSlackSelectionApplier {
            applied: Mutex::new(Vec::new()),
            should_fail: true,
        });
        let (handler, ..) = make_handler_with_slack(None, None, Some(applier));

        let result = handler
            .handle(Command::ApplySlackSelection {
                channel_ids: vec![],
                watched_user_ids: vec![],
            })
            .await;
        assert!(result.is_err());
    }

    // Reuses MockConnector above to prove a second source (GitHub) works
    // through the exact same registry/dispatch path as Slack did -- the
    // whole point of generalizing Command::Connect (step11.md).
    #[tokio::test]
    async fn connect_works_for_a_second_registered_source() {
        let connector = Arc::new(MockConnector {
            connected: Mutex::new(Vec::new()),
            should_fail: false,
        });
        let mut connectors: HashMap<IntegrationSource, Arc<dyn IntegrationConnector>> =
            HashMap::new();
        connectors.insert(
            IntegrationSource::GitHub,
            Arc::clone(&connector) as Arc<dyn IntegrationConnector>,
        );
        let (handler, ..) = make_handler_with(None, None, connectors, HashMap::new());

        handler
            .handle(Command::Connect {
                source: IntegrationSource::GitHub,
                token: "ghp_from_ui".into(),
            })
            .await
            .unwrap();

        assert_eq!(
            connector.connected.lock().await.as_slice(),
            ["ghp_from_ui".to_string()]
        );
    }

    struct MockSelectionApplier {
        applied: Mutex<Vec<Vec<String>>>,
        should_fail: bool,
    }

    #[async_trait]
    impl SelectionApplier for MockSelectionApplier {
        async fn apply(&self, _event_bus: Arc<dyn EventBus>, items: Vec<String>) -> Result<()> {
            if self.should_fail {
                return Err(common::WorkspaceError::Integration("boom".into()));
            }
            self.applied.lock().await.push(items);
            Ok(())
        }
    }

    #[tokio::test]
    async fn apply_selection_errors_without_a_matching_adapter() {
        let (handler, ..) = make_handler();
        let result = handler
            .handle(Command::ApplySelection {
                source: IntegrationSource::GitHub,
                items: vec!["owner/repo".into()],
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn apply_selection_delegates_to_the_applier_registered_for_that_source() {
        let applier = Arc::new(MockSelectionApplier {
            applied: Mutex::new(Vec::new()),
            should_fail: false,
        });
        let mut selection_appliers: HashMap<IntegrationSource, Arc<dyn SelectionApplier>> =
            HashMap::new();
        selection_appliers.insert(
            IntegrationSource::GitHub,
            Arc::clone(&applier) as Arc<dyn SelectionApplier>,
        );
        let (handler, ..) = make_handler_with(None, None, HashMap::new(), selection_appliers);

        handler
            .handle(Command::ApplySelection {
                source: IntegrationSource::GitHub,
                items: vec!["owner/repo".into()],
            })
            .await
            .unwrap();

        assert_eq!(
            applier.applied.lock().await.as_slice(),
            [vec!["owner/repo".to_string()]]
        );
    }

    #[tokio::test]
    async fn apply_selection_propagates_applier_errors() {
        let applier = Arc::new(MockSelectionApplier {
            applied: Mutex::new(Vec::new()),
            should_fail: true,
        });
        let mut selection_appliers: HashMap<IntegrationSource, Arc<dyn SelectionApplier>> =
            HashMap::new();
        selection_appliers.insert(IntegrationSource::GitHub, applier);
        let (handler, ..) = make_handler_with(None, None, HashMap::new(), selection_appliers);

        let result = handler
            .handle(Command::ApplySelection {
                source: IntegrationSource::GitHub,
                items: vec![],
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn sync_all_adapters_is_a_noop_ok() {
        let (handler, ..) = make_handler();
        handler.handle(Command::SyncAllAdapters).await.unwrap();
    }

    #[tokio::test]
    async fn dispatcher_delegates_to_handler() {
        let (handler, presence, ..) = make_handler();
        let dispatcher = InMemoryCommandDispatcher::new(handler);

        dispatcher
            .dispatch(Command::SetPresence {
                status: PresenceStatus::Active,
                custom_text: None,
            })
            .await
            .unwrap();

        assert_eq!(presence.saved.lock().await.len(), 1);
    }

    fn sample_notification(title: &str, timestamp_ms: u64) -> NotificationItem {
        NotificationItem {
            id: NotificationId(Uuid::new_v4()),
            source: domain::IntegrationSource::Slack,
            title: title.into(),
            body: String::new(),
            timestamp_ms,
            priority: domain::PriorityLevel::Medium,
            is_read: false,
            action_link: None,
        }
    }

    #[tokio::test]
    async fn projector_populates_initial_model_from_storage() {
        let presence_repo: Arc<dyn PresenceRepository> = Arc::new(MockPresenceRepo::default());
        presence_repo
            .save_presence(&MemberPresence {
                user_id: UserId("u1".into()),
                display_name: "Alice".into(),
                status: PresenceStatus::Active,
                custom_status_text: None,
                last_updated_ms: 0,
            })
            .await
            .unwrap();
        let notification_repo: Arc<dyn NotificationRepository> =
            Arc::new(MockNotificationRepo::default());
        notification_repo
            .save(&sample_notification("first", 100))
            .await
            .unwrap();

        let (_projector, model) = Projector::new(&presence_repo, &notification_repo)
            .await
            .unwrap();

        let snapshot = model.read().await;
        assert_eq!(snapshot.team_presence.len(), 1);
        assert_eq!(snapshot.unread_notifications.len(), 1);
    }

    #[tokio::test]
    async fn projector_upserts_presence_on_event() {
        let presence_repo: Arc<dyn PresenceRepository> = Arc::new(MockPresenceRepo::default());
        let notification_repo: Arc<dyn NotificationRepository> =
            Arc::new(MockNotificationRepo::default());
        let (projector, model) = Projector::new(&presence_repo, &notification_repo)
            .await
            .unwrap();

        let presence = MemberPresence {
            user_id: UserId("u1".into()),
            display_name: "Alice".into(),
            status: PresenceStatus::Active,
            custom_status_text: None,
            last_updated_ms: 1,
        };
        projector
            .handle(Event::SlackPresenceChanged(presence.clone()))
            .await
            .unwrap();
        assert_eq!(model.read().await.team_presence.len(), 1);

        // Same user_id again must update in place, not duplicate.
        projector
            .handle(Event::SlackPresenceChanged(MemberPresence {
                status: PresenceStatus::Away,
                ..presence
            }))
            .await
            .unwrap();
        let snapshot = model.read().await;
        assert_eq!(snapshot.team_presence.len(), 1);
        assert_eq!(snapshot.team_presence[0].status, PresenceStatus::Away);
    }

    #[tokio::test]
    async fn projector_upserts_and_sorts_notifications_on_event() {
        let presence_repo: Arc<dyn PresenceRepository> = Arc::new(MockPresenceRepo::default());
        let notification_repo: Arc<dyn NotificationRepository> =
            Arc::new(MockNotificationRepo::default());
        let (projector, model) = Projector::new(&presence_repo, &notification_repo)
            .await
            .unwrap();

        let older = sample_notification("older", 100);
        let newer = sample_notification("newer", 200);
        projector
            .handle(Event::SlackMessageReceived(older.clone()))
            .await
            .unwrap();
        projector
            .handle(Event::GitHubPRCreated(newer.clone()))
            .await
            .unwrap();

        let snapshot = model.read().await;
        assert_eq!(snapshot.unread_notifications.len(), 2);
        // Newest first.
        assert_eq!(snapshot.unread_notifications[0].title, "newer");

        drop(snapshot);

        // Re-delivering the same id must update in place, not duplicate.
        let mut updated = newer;
        updated.title = "newer-updated".into();
        projector
            .handle(Event::GitHubPRCreated(updated))
            .await
            .unwrap();
        let snapshot = model.read().await;
        assert_eq!(snapshot.unread_notifications.len(), 2);
        assert_eq!(snapshot.unread_notifications[0].title, "newer-updated");
    }
}
