//! Scheduler Domain executing recurring timers.

use common::Result;
use events::EventBus;
use std::sync::Arc;

/// Dynamic Agenda scheduler managing meetings and Pomodoros.
#[allow(dead_code)]
pub struct AgendaScheduler {
    event_bus: Arc<dyn EventBus>,
}

impl AgendaScheduler {
    /// Create new scheduler coordinator.
    #[must_use]
    pub fn new(event_bus: Arc<dyn EventBus>) -> Self {
        Self { event_bus }
    }

    /// Spawns background loop evaluating alarm deadlines.
    pub async fn run_loop(&self) -> Result<()> {
        // In skeleton, we simply print logs
        tracing::info!("Scheduler time loop started.");
        Ok(())
    }
}
