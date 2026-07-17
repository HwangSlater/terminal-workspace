//! CQRS Command definitions and Command Handler traits.

use async_trait::async_trait;
use common::{Result, WorkspaceError};
use domain::{
    MemberPresence, NotificationId, NotificationRepository, PresenceRepository, PresenceStatus,
    UserId,
};
use events::{Event, EventBus};
use logging::{spans::command_span, TraceContext};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::Instrument;

/// Strongly-typed CQRS system write commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Mark notification as read.
    MarkNotificationRead {
        /// Unique identifier.
        id: NotificationId,
    },

    /// Force synchronization check on all active integration adapters.
    SyncAllAdapters,
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
}

impl WorkspaceCommandHandler {
    /// Construct a handler wired to the given repositories and event bus.
    #[must_use]
    pub fn new(
        presence_repo: Arc<dyn PresenceRepository>,
        notification_repo: Arc<dyn NotificationRepository>,
        event_bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            presence_repo,
            notification_repo,
            event_bus,
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
            Command::SendSlackMessage { .. } => Err(WorkspaceError::Integration(
                "Slack integration not yet implemented".into(),
            )),
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
        marked_read: Mutex<Vec<NotificationId>>,
    }

    #[async_trait]
    impl NotificationRepository for MockNotificationRepo {
        async fn save(&self, _item: &NotificationItem) -> Result<()> {
            Ok(())
        }

        async fn find_by_id(&self, _id: &NotificationId) -> Result<Option<NotificationItem>> {
            Ok(None)
        }

        async fn fetch_unread(&self) -> Result<Vec<NotificationItem>> {
            Ok(Vec::new())
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
        let presence = Arc::new(MockPresenceRepo::default());
        let notifications = Arc::new(MockNotificationRepo::default());
        let bus = Arc::new(InProcessEventBus::new(10));
        let handler = Arc::new(WorkspaceCommandHandler::new(
            Arc::clone(&presence) as Arc<dyn PresenceRepository>,
            Arc::clone(&notifications) as Arc<dyn NotificationRepository>,
            Arc::clone(&bus) as Arc<dyn EventBus>,
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
}
