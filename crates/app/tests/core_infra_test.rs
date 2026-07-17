use async_trait::async_trait;
use common::Result;
use config::ConfigBuilder;
use domain::{IntegrationSource, NotificationItem, PriorityLevel};
use events::{Event, EventBus, EventDispatcher, EventHandler, InProcessEventBus};
use logging::{spans::event_span, TraceContext};
use registry::{CommandRegistry, InMemoryCommandRegistry, RegisteredCommand};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::Instrument;
use uuid::Uuid;

// Mock EventHandler tracking received events. Emits a log line (proxy for
// "Console Output" in the vertical-slice flow: Mock Command -> EventBus ->
// Registry -> Logging -> Console Output) from inside an `event_span`.
struct MockEventHandler {
    received: Arc<Mutex<Vec<Event>>>,
}

#[async_trait]
impl EventHandler for MockEventHandler {
    async fn handle(&self, event: Event) -> Result<()> {
        let ctx = TraceContext::new();
        let span = event_span("SlackMessageReceived", &ctx);

        async {
            tracing::info!("mock handler routed event to console output");
            let mut list = self.received.lock().await;
            list.push(event);
            Ok(())
        }
        .instrument(span)
        .await
    }
}

#[tokio::test]
async fn test_vertical_slice_infra_flow() -> Result<()> {
    // 0. Config + Logging: prove the Phase 2 primitives compose end-to-end
    //    without any real integration, per step2_feedback.md item 7.
    let config = ConfigBuilder::new().build()?;
    assert_eq!(config.core.log_level, "info");
    // Safe to call from multiple tests/processes: ignores "already initialized".
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // 1. Setup Event Bus & Dispatcher
    let event_bus = Arc::new(InProcessEventBus::new(10));
    let dispatcher = EventDispatcher::new(Arc::clone(&event_bus) as Arc<dyn EventBus>);

    let received_events = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(MockEventHandler {
        received: Arc::clone(&received_events),
    });

    dispatcher
        .register_handler(handler as Arc<dyn EventHandler>)
        .await;
    dispatcher.start();

    // 2. Setup Command Registry with a Mock Command
    let cmd_registry = InMemoryCommandRegistry::new();
    cmd_registry
        .register_command(RegisteredCommand {
            name: "test-cmd".into(),
            description: "Integration test mock command".into(),
        })
        .await?;

    // Assert registry configuration
    let list = cmd_registry.list_commands().await?;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "test-cmd");

    // 3. Publish mock event through Event Bus
    let item = NotificationItem {
        id: domain::NotificationId(Uuid::new_v4()),
        source: IntegrationSource::Slack,
        title: "Build Succeeded".into(),
        body: "Phase 1 and 2 integration flows run smoothly.".into(),
        timestamp_ms: 1716373200,
        priority: PriorityLevel::Medium,
        is_read: false,
        action_link: None,
    };

    event_bus.publish(Event::SlackMessageReceived(item)).await?;

    // Give asynchronous tasks time to route
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    // 4. Assert handler intercepted the event (and, by not panicking inside
    //    the span/log call above, that Config + Logging + EventBus +
    //    Registry all composed successfully in one run).
    let events = received_events.lock().await;
    assert_eq!(events.len(), 1);

    if let Event::SlackMessageReceived(ref inner_item) = events[0] {
        assert_eq!(inner_item.title, "Build Succeeded");
        assert_eq!(inner_item.source, IntegrationSource::Slack);
    } else {
        panic!("Incorrect event routed through dispatcher");
    }

    Ok(())
}
