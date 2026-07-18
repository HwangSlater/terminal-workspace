use async_trait::async_trait;
use commands::{Command, CommandDispatcher, InMemoryCommandDispatcher, WorkspaceCommandHandler};
use common::Result;
use domain::{
    FailedEventRepository, IntegrationSource, NotificationId, NotificationItem,
    NotificationRepository, PresenceRepository, PresenceStatus, PriorityLevel,
};
use events::{Event, EventBus, EventDispatcher, EventHandler, InProcessEventBus};
use std::sync::Arc;
use storage::RedbStorageBackend;
use tokio::sync::Mutex;
use uuid::Uuid;

struct RecordingHandler {
    received: Arc<Mutex<Vec<Event>>>,
}

#[async_trait]
impl EventHandler for RecordingHandler {
    async fn handle(&self, event: Event) -> Result<()> {
        self.received.lock().await.push(event);
        Ok(())
    }
}

/// Proves the Phase 3 write path end-to-end without any real integration:
/// Command -> WorkspaceCommandHandler -> Storage (temp-file redb) -> Event
/// -> EventDispatcher. Continues Phase 2's `core_infra_test.rs` pattern.
#[tokio::test]
async fn cqrs_write_path_flows_through_storage_and_events() -> Result<()> {
    let temp_db_path =
        std::env::temp_dir().join(format!("tw_cqrs_slice_test_{}.redb", Uuid::new_v4()));
    let storage = Arc::new(RedbStorageBackend::open(&temp_db_path).await?);
    let event_bus = Arc::new(InProcessEventBus::new(16));

    let received = Arc::new(Mutex::new(Vec::new()));
    let dispatcher_event = EventDispatcher::new(Arc::clone(&event_bus) as Arc<dyn EventBus>)
        .with_dlq(Arc::clone(&storage) as Arc<dyn FailedEventRepository>);
    dispatcher_event
        .register_handler(Arc::new(RecordingHandler {
            received: Arc::clone(&received),
        }))
        .await;
    dispatcher_event.start();

    let handler = Arc::new(WorkspaceCommandHandler::new(
        Arc::clone(&storage) as Arc<dyn PresenceRepository>,
        Arc::clone(&storage) as Arc<dyn NotificationRepository>,
        Arc::clone(&event_bus) as Arc<dyn EventBus>,
        None,
        None,
    ));
    let command_dispatcher = InMemoryCommandDispatcher::new(handler);

    // 1. SetPresence persists to storage and publishes an Event.
    command_dispatcher
        .dispatch(Command::SetPresence {
            status: PresenceStatus::Away,
            custom_text: Some("in a meeting".into()),
        })
        .await?;

    let all_presence = storage.fetch_all().await?;
    assert_eq!(all_presence.len(), 1);
    assert_eq!(all_presence[0].status, PresenceStatus::Away);

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let events = received.lock().await;
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::SlackPresenceChanged(p) => {
            assert_eq!(p.custom_status_text.as_deref(), Some("in a meeting"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    drop(events);

    // 2. MarkNotificationRead persists to storage.
    let notification = NotificationItem {
        id: NotificationId(Uuid::new_v4()),
        source: IntegrationSource::GitHub,
        title: "PR merged".into(),
        body: "".into(),
        timestamp_ms: 0,
        priority: PriorityLevel::Low,
        is_read: false,
        action_link: None,
    };
    storage.save(&notification).await?;
    assert_eq!(storage.fetch_unread().await?.len(), 1);

    command_dispatcher
        .dispatch(Command::MarkNotificationRead {
            id: notification.id.clone(),
        })
        .await?;
    assert!(storage.fetch_unread().await?.is_empty());

    // 3. SendSlackMessage fails honestly: no Slack integration exists yet.
    let result = command_dispatcher
        .dispatch(Command::SendSlackMessage {
            channel_id: "C1".into(),
            text: "hello".into(),
        })
        .await;
    assert!(result.is_err());

    Ok(())
}
