//! Presentational Terminal User Interface (TUI) stubs.

use common::Result;
use registry::UiRegistry;
use std::sync::Arc;

/// Dynamic TUI App engine orchestrating Crossterm events and Ratatui frames.
#[allow(dead_code)]
pub struct TuiRenderer {
    ui_registry: Arc<dyn UiRegistry>,
}

impl TuiRenderer {
    /// Create new renderer wrapper.
    #[must_use]
    pub fn new(ui_registry: Arc<dyn UiRegistry>) -> Self {
        Self { ui_registry }
    }

    /// Spawn main Crossterm polling loop and render frames.
    pub async fn run_loop(&self) -> Result<()> {
        // In skeleton, we simply print startup logs
        tracing::info!("TUI viewport loop started.");
        Ok(())
    }
}
