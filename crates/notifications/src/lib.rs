//! Desktop (OS-level) notifications for the subset of `Event`s a user
//! needs to know about even when Terminal Workspace isn't the focused
//! window or terminal -- `step21.md`. Before this crate, nothing told a
//! user about a Slack DM, a GitHub review request, a Calendar reminder, or
//! a finished Pomodoro session unless they were already looking at the TUI
//! (or ran `termws status`, a pull, not a push).

use async_trait::async_trait;
use common::Result;
use events::{Event, EventHandler};

/// Which four `Event` variants surface as a desktop notification, and the
/// title/body to show for each (`step21.md` Decision 2). `Event` is frozen
/// by Architecture Freeze v1 (`development.md` §3), so this reuses the
/// existing variants rather than adding a new one. Deliberately narrow:
/// presence/connection-status churn and plugin events are excluded as too
/// frequent or too low-signal to interrupt another app for.
fn notification_for_event(event: &Event) -> Option<(String, String)> {
    match event {
        Event::SlackMessageReceived(item) => {
            Some((format!("슬랙: {}", item.title), item.body.clone()))
        }
        Event::GitHubPRCreated(item) => {
            Some((format!("깃허브: {}", item.title), item.body.clone()))
        }
        Event::CalendarReminderTriggered(item) => {
            Some((format!("캘린더: {}", item.title), item.body.clone()))
        }
        // Covers Pomodoro session-end (step18.md) and any other
        // best-effort system message published through this variant.
        Event::SystemAlert(message) => Some(("Terminal Workspace".to_string(), message.clone())),
        Event::SlackPresenceChanged(_)
        | Event::IntegrationStatusChanged { .. }
        | Event::PluginCustomEvent { .. } => None,
    }
}

/// Registered on the same `EventDispatcher` every other `EventHandler` is
/// (`crates/app/src/main.rs`), alongside `Projector`/`PluginHostManager` --
/// no new wiring shape. Always-on for the four event kinds
/// `notification_for_event` recognizes; no mute/quiet-hours control in
/// this phase (`step21.md` Decision 5).
#[derive(Debug, Default)]
pub struct DesktopNotifier;

impl DesktopNotifier {
    /// Creates a new notifier. Holds no state -- every notification is
    /// built fresh from the `Event` being handled.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EventHandler for DesktopNotifier {
    async fn handle(&self, event: Event) -> Result<()> {
        let Some((title, body)) = notification_for_event(&event) else {
            return Ok(());
        };
        // A failed OS notification (no notification daemon on a headless
        // Linux box, permission denied on macOS, ...) must never be fatal
        // to the app -- `step21.md` Decision 3, the same "best-effort side
        // channel" treatment the Pomodoro terminal bell already gets.
        if let Err(e) = notify_rust::Notification::new()
            .summary(&title)
            .body(&body)
            .show()
        {
            tracing::warn!("desktop notification failed: {e}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{
        IntegrationSource, MemberPresence, NotificationId, NotificationItem, PresenceStatus,
        PriorityLevel, UserId,
    };
    use events::IntegrationConnectionStatus;
    use uuid::Uuid;

    fn sample_item(title: &str, body: &str) -> NotificationItem {
        NotificationItem {
            id: NotificationId(Uuid::new_v4()),
            source: IntegrationSource::Slack,
            title: title.to_string(),
            body: body.to_string(),
            timestamp_ms: 0,
            priority: PriorityLevel::Medium,
            is_read: false,
            action_link: None,
        }
    }

    #[test]
    fn slack_message_becomes_a_labeled_notification() {
        let event = Event::SlackMessageReceived(sample_item("PR approved", "great work"));
        let (title, body) = notification_for_event(&event).unwrap();
        assert_eq!(title, "슬랙: PR approved");
        assert_eq!(body, "great work");
    }

    #[test]
    fn github_pr_becomes_a_labeled_notification() {
        let event = Event::GitHubPRCreated(sample_item("Review requested", "repo/wasm#42"));
        let (title, body) = notification_for_event(&event).unwrap();
        assert_eq!(title, "깃허브: Review requested");
        assert_eq!(body, "repo/wasm#42");
    }

    #[test]
    fn calendar_reminder_becomes_a_labeled_notification() {
        let event = Event::CalendarReminderTriggered(sample_item("Design Review", "in 10 min"));
        let (title, body) = notification_for_event(&event).unwrap();
        assert_eq!(title, "캘린더: Design Review");
        assert_eq!(body, "in 10 min");
    }

    #[test]
    fn system_alert_passes_the_message_through_as_the_body() {
        let event = Event::SystemAlert("Work session complete -- take a break!".to_string());
        let (title, body) = notification_for_event(&event).unwrap();
        assert_eq!(title, "Terminal Workspace");
        assert_eq!(body, "Work session complete -- take a break!");
    }

    /// Real regression guard for Decision 2's scope: presence churn and
    /// connection-status flapping must not surface as desktop
    /// notifications -- these fire far more often than the four events
    /// above and would make notifications noise instead of signal.
    #[test]
    fn presence_and_connection_status_events_produce_no_notification() {
        let presence = MemberPresence {
            user_id: UserId("u1".into()),
            display_name: "Alice".into(),
            status: PresenceStatus::Active,
            custom_status_text: None,
            last_updated_ms: 0,
        };
        assert!(notification_for_event(&Event::SlackPresenceChanged(presence)).is_none());
    }

    #[test]
    fn integration_status_changed_produces_no_notification() {
        let event = Event::IntegrationStatusChanged {
            source: IntegrationSource::GitHub,
            status: IntegrationConnectionStatus::Connected,
        };
        assert!(notification_for_event(&event).is_none());
    }

    #[test]
    fn plugin_custom_event_produces_no_notification() {
        let event = Event::PluginCustomEvent {
            plugin_id: "example".to_string(),
            payload_json: "{}".to_string(),
        };
        assert!(notification_for_event(&event).is_none());
    }
}
