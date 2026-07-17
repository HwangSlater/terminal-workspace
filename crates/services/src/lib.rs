//! Core Domain Services orchestrating business policies.

use async_trait::async_trait;
use common::Result;
use domain::{MemberPresence, NotificationItem};

/// Domain Service managing user presence rules.
#[async_trait]
pub trait PresenceDomainService: Send + Sync {
    /// Update presence status and evaluate auto-away policies.
    async fn update_presence(&self, presence: MemberPresence) -> Result<()>;
}

/// Domain Service executing rules on incoming notifications.
#[async_trait]
pub trait NotificationDomainService: Send + Sync {
    /// Ingest a notification item, process deduplication, and check rules.
    async fn process_incoming(&self, item: NotificationItem) -> Result<()>;
}
