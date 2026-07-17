//! Unified Command, Service, and UI Registries implementation using in-memory locks.

use async_trait::async_trait;
use common::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Registered Command definition payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredCommand {
    /// Command string pattern (e.g. "slack-send").
    pub name: String,
    /// Helper text.
    pub description: String,
}

/// Registry capturing execution target endpoints.
#[async_trait]
pub trait CommandRegistry: Send + Sync {
    /// Register dynamic command.
    async fn register_command(&self, command: RegisteredCommand) -> Result<()>;

    /// Retrieve list of all commands.
    async fn list_commands(&self) -> Result<Vec<RegisteredCommand>>;

    /// Unregister command.
    async fn remove_command(&self, name: &str) -> Result<()>;
}

/// InMemory implementation of CommandRegistry.
pub struct InMemoryCommandRegistry {
    commands: RwLock<HashMap<String, RegisteredCommand>>,
}

impl InMemoryCommandRegistry {
    /// Create new empty command registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            commands: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryCommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CommandRegistry for InMemoryCommandRegistry {
    async fn register_command(&self, command: RegisteredCommand) -> Result<()> {
        let mut map = self.commands.write().await;
        map.insert(command.name.clone(), command);
        Ok(())
    }

    async fn list_commands(&self) -> Result<Vec<RegisteredCommand>> {
        let map = self.commands.read().await;
        Ok(map.values().cloned().collect())
    }

    async fn remove_command(&self, name: &str) -> Result<()> {
        let mut map = self.commands.write().await;
        map.remove(name);
        Ok(())
    }
}

/// Dynamic GUI slot location hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UiDockSlot {
    /// Left drawer.
    Left,
    /// Main center container.
    Center,
    /// Right panel context.
    Right,
    /// Log stream bottom drawer.
    Bottom,
}

/// Registry mapping layout widgets to UI viewport frames.
#[async_trait]
pub trait UiRegistry: Send + Sync {
    /// Dock dynamic widget target view.
    async fn register_panel(&self, panel_id: &str, target_slot: UiDockSlot) -> Result<()>;

    /// Remove panel widget link.
    async fn unregister_panel(&self, panel_id: &str) -> Result<()>;

    /// Get all views bound to slot.
    async fn list_slot_panels(&self, slot: UiDockSlot) -> Result<Vec<String>>;
}

/// InMemory implementation of UiRegistry.
pub struct InMemoryUiRegistry {
    docks: RwLock<HashMap<UiDockSlot, Vec<String>>>,
}

impl InMemoryUiRegistry {
    /// Create new empty UI registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            docks: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryUiRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl UiRegistry for InMemoryUiRegistry {
    async fn register_panel(&self, panel_id: &str, target_slot: UiDockSlot) -> Result<()> {
        let mut map = self.docks.write().await;
        let list = map.entry(target_slot).or_default();
        if !list.contains(&panel_id.to_string()) {
            list.push(panel_id.to_string());
        }
        Ok(())
    }

    async fn unregister_panel(&self, panel_id: &str) -> Result<()> {
        let mut map = self.docks.write().await;
        for list in map.values_mut() {
            list.retain(|id| id != panel_id);
        }
        Ok(())
    }

    async fn list_slot_panels(&self, slot: UiDockSlot) -> Result<Vec<String>> {
        let map = self.docks.read().await;
        Ok(map.get(&slot).cloned().unwrap_or_default())
    }
}

/// Dynamic shared Service context.
#[async_trait]
pub trait SharedService: Send + Sync {
    /// Call implementation interface directly.
    async fn call_service(&self, method: &str, params_json: &str) -> Result<String>;
}

/// Registry routing queries to core and plugin microservice adapters.
#[async_trait]
pub trait ServiceRegistry: Send + Sync {
    /// Register dynamic service adapter.
    async fn register_service(&self, name: &str, service: Arc<dyn SharedService>) -> Result<()>;

    /// Query registered service context.
    async fn get_service(&self, name: &str) -> Result<Option<Arc<dyn SharedService>>>;
}

/// InMemory implementation of ServiceRegistry.
pub struct InMemoryServiceRegistry {
    services: RwLock<HashMap<String, Arc<dyn SharedService>>>,
}

impl InMemoryServiceRegistry {
    /// Create new empty service registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            services: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ServiceRegistry for InMemoryServiceRegistry {
    async fn register_service(&self, name: &str, service: Arc<dyn SharedService>) -> Result<()> {
        let mut map = self.services.write().await;
        map.insert(name.to_string(), service);
        Ok(())
    }

    async fn get_service(&self, name: &str) -> Result<Option<Arc<dyn SharedService>>> {
        let map = self.services.read().await;
        Ok(map.get(name).cloned())
    }
}
