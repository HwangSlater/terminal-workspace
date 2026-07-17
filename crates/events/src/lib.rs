//! Event Bus and Event Dispatcher implementation using Tokio broadcast channels.

use async_trait::async_trait;
use common::Result;
use domain::{FailedEventRecord, FailedEventRepository, MemberPresence, NotificationItem};
use logging::{spans::event_span, TraceContext};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, RwLock};
use tracing::Instrument;
use uuid::Uuid;

/// Strongly-typed platform Event Enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    /// Received a message from Slack Integration.
    SlackMessageReceived(NotificationItem),

    /// User status changes from Slack.
    SlackPresenceChanged(MemberPresence),

    /// Pull Request alert from GitHub Integration.
    GitHubPRCreated(NotificationItem),

    /// Calendar timer reminder trigger.
    CalendarReminderTriggered(NotificationItem),

    /// System alert indicating warnings or fatal conditions.
    SystemAlert(String),

    /// Dynamic event dispatched from a plugin guest workspace.
    PluginCustomEvent {
        /// Target plugin identifier.
        plugin_id: String,
        /// Serialized event payload.
        payload_json: String,
    },
}

/// Abstract Event Handler processing inbound Event Enum messages.
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// Process the captured event asynchronously.
    async fn handle(&self, event: Event) -> Result<()>;
}

/// Central broker enabling 1-to-many event routing.
#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publish event to subscribers.
    async fn publish(&self, event: Event) -> Result<()>;

    /// Subscribe to the raw event broadcast channel.
    fn subscribe(&self) -> broadcast::Receiver<Event>;
}

/// In-process EventBus implementation using Tokio broadcast.
pub struct InProcessEventBus {
    sender: broadcast::Sender<Event>,
}

impl InProcessEventBus {
    /// Create new event bus with specified buffer queue size.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }
}

#[async_trait]
impl EventBus for InProcessEventBus {
    async fn publish(&self, event: Event) -> Result<()> {
        // If there are zero subscribers, broadcast returns a SendError, which we ignore.
        let _ = self.sender.send(event);
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}

/// Dispatcher pulling events from EventBus and distributing them to specialized handlers.
pub struct EventDispatcher {
    event_bus: Arc<dyn EventBus>,
    handlers: Arc<RwLock<Vec<Arc<dyn EventHandler>>>>,
    dlq: Option<Arc<dyn FailedEventRepository>>,
}

impl EventDispatcher {
    /// Create dispatcher binding to target Event Bus. Without [`Self::with_dlq`],
    /// a handler failure is logged once and dropped (Phase 2 behavior).
    #[must_use]
    pub fn new(event_bus: Arc<dyn EventBus>) -> Self {
        Self {
            event_bus,
            handlers: Arc::new(RwLock::new(Vec::new())),
            dlq: None,
        }
    }

    /// Opt this dispatcher into exponential-backoff retry (see
    /// `docs/02-architecture/events.md` "Retry Policy & Backoff") and Dead Letter Queue
    /// persistence via `repo` once retries are exhausted. See
    /// `docs/06-development/decisions/0003-event-bus.md`'s Phase 3 amendment.
    #[must_use]
    pub fn with_dlq(mut self, repo: Arc<dyn FailedEventRepository>) -> Self {
        self.dlq = Some(repo);
        self
    }

    /// Register dynamic event subscriber.
    pub async fn register_handler(&self, handler: Arc<dyn EventHandler>) {
        let mut list = self.handlers.write().await;
        list.push(handler);
    }

    /// Starts the background subscriber loop pulling events and routing to handlers.
    pub fn start(&self) {
        let mut rx = self.event_bus.subscribe();
        let handlers = Arc::clone(&self.handlers);
        let dlq = self.dlq.clone();

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let active_handlers = handlers.read().await;
                        for handler in active_handlers.iter() {
                            let h = Arc::clone(handler);
                            let ev = event.clone();
                            match dlq.clone() {
                                Some(dlq) => {
                                    tokio::spawn(async move {
                                        dispatch_with_retry(h, ev, dlq).await;
                                    });
                                }
                                None => {
                                    tokio::spawn(async move {
                                        if let Err(e) = h.handle(ev).await {
                                            tracing::error!(
                                                "Event handler execution failed: {:?}",
                                                e
                                            );
                                        }
                                    });
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!("Event dispatcher lagged by {} messages", skipped);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!("Event bus channel closed. Exiting dispatcher loop.");
                        break;
                    }
                }
            }
        });
    }
}

const MAX_RETRY_ATTEMPTS: u32 = 5;
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

fn event_type_name(event: &Event) -> &'static str {
    match event {
        Event::SlackMessageReceived(_) => "SlackMessageReceived",
        Event::SlackPresenceChanged(_) => "SlackPresenceChanged",
        Event::GitHubPRCreated(_) => "GitHubPRCreated",
        Event::CalendarReminderTriggered(_) => "CalendarReminderTriggered",
        Event::SystemAlert(_) => "SystemAlert",
        Event::PluginCustomEvent { .. } => "PluginCustomEvent",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Run `handler` against `event` with exponential backoff retry (per
/// `docs/02-architecture/events.md`), persisting a [`FailedEventRecord`] to `dlq` if every
/// attempt fails.
async fn dispatch_with_retry(
    handler: Arc<dyn EventHandler>,
    event: Event,
    dlq: Arc<dyn FailedEventRepository>,
) {
    let type_name = event_type_name(&event);
    let ctx = TraceContext::new();
    let span = event_span(type_name, &ctx);

    async move {
        let mut delay = INITIAL_RETRY_DELAY;
        let mut last_error = None;

        for attempt in 1..=MAX_RETRY_ATTEMPTS {
            match handler.handle(event.clone()).await {
                Ok(()) => return,
                Err(e) => {
                    tracing::warn!(
                        attempt,
                        max_attempts = MAX_RETRY_ATTEMPTS,
                        "Event handler attempt failed: {:?}",
                        e
                    );
                    last_error = Some(e.to_string());
                    if attempt < MAX_RETRY_ATTEMPTS {
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(MAX_RETRY_DELAY);
                    }
                }
            }
        }

        let error_message = last_error.unwrap_or_default();
        tracing::error!(%error_message, "Event handler exhausted all retry attempts; writing to DLQ");

        // `producer` would ideally identify the originating adapter, but the
        // frozen `Event` enum carries no such field yet; the event type name
        // is the closest available identifier.
        let record = FailedEventRecord {
            id: Uuid::new_v4(),
            event_type: type_name.to_string(),
            producer: type_name.to_string(),
            payload_json: serde_json::to_string(&event).unwrap_or_default(),
            error_message,
            retry_count: MAX_RETRY_ATTEMPTS,
            failed_at_ms: now_ms(),
        };

        if let Err(e) = dlq.save_failed(&record).await {
            tracing::error!("Failed to persist DLQ record: {:?}", e);
        }
    }
    .instrument(span)
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::WorkspaceError;
    use tokio::sync::Mutex;

    struct AlwaysFailHandler;

    #[async_trait]
    impl EventHandler for AlwaysFailHandler {
        async fn handle(&self, _event: Event) -> Result<()> {
            Err(WorkspaceError::Internal("boom".into()))
        }
    }

    struct SucceedsOnThirdTry {
        attempts: Mutex<u32>,
    }

    #[async_trait]
    impl EventHandler for SucceedsOnThirdTry {
        async fn handle(&self, _event: Event) -> Result<()> {
            let mut n = self.attempts.lock().await;
            *n += 1;
            if *n >= 3 {
                Ok(())
            } else {
                Err(WorkspaceError::Internal("retry me".into()))
            }
        }
    }

    #[derive(Default)]
    struct MockDlq {
        saved: Mutex<Vec<FailedEventRecord>>,
    }

    #[async_trait]
    impl FailedEventRepository for MockDlq {
        async fn save_failed(&self, record: &FailedEventRecord) -> Result<()> {
            self.saved.lock().await.push(record.clone());
            Ok(())
        }

        async fn list_failed(&self) -> Result<Vec<FailedEventRecord>> {
            Ok(self.saved.lock().await.clone())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn exhausted_retries_write_to_dlq() {
        let dlq = Arc::new(MockDlq::default());

        dispatch_with_retry(
            Arc::new(AlwaysFailHandler),
            Event::SystemAlert("test".into()),
            Arc::clone(&dlq) as Arc<dyn FailedEventRepository>,
        )
        .await;

        let saved = dlq.saved.lock().await;
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].retry_count, MAX_RETRY_ATTEMPTS);
        assert_eq!(saved[0].event_type, "SystemAlert");
    }

    #[tokio::test(start_paused = true)]
    async fn success_before_exhausting_retries_skips_dlq() {
        let dlq = Arc::new(MockDlq::default());
        let handler = Arc::new(SucceedsOnThirdTry {
            attempts: Mutex::new(0),
        });

        dispatch_with_retry(
            handler,
            Event::SystemAlert("test".into()),
            Arc::clone(&dlq) as Arc<dyn FailedEventRepository>,
        )
        .await;

        assert!(dlq.saved.lock().await.is_empty());
    }
}
