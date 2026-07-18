//! Host runtime execution environment for Guest Plugins (`step14.md`,
//! ADR-0002/0009/0017). Discovers `<plugin-id>.wasm` Component-Model
//! binaries from a configured directory (filtered by an explicit
//! allow-list -- `step14.md` Decision 4, default-off/opt-in), sandboxes
//! each with a fuel budget (1,000,000 instructions per call) and a 64MB
//! memory limit (`docs/04-extensions/plugin-lifecycle.md` §3), and
//! forwards `Event`s from the shared `EventBus` into each guest's
//! `on-event` export. A trapped guest (fuel exhaustion, OOM, panic) is
//! caught and the plugin instance is dropped (`Suspended`, per the
//! lifecycle state diagram) -- it never takes down the host process
//! (ADR-0002).

use async_trait::async_trait;
use common::{Result, WorkspaceError};
use events::{Event, EventBus, EventHandler};
use plugin_sdk::PluginCapability;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder};

// `wasmtime::component::bindgen!`'s generated code refers to a bare,
// unqualified `Result` internally, which would collide with this crate's
// `use common::Result` (a 1-generic-argument alias) if both lived at
// crate-root scope -- scoping the macro into its own module keeps the two
// `Result`s from colliding (confirmed via a real E0107 compile error while
// wiring this up, `step14.md` Implementation Notes).
mod bindings {
    wasmtime::component::bindgen!({
        world: "developer-workspace-plugin",
        path: "../plugin-sdk/wit/plugin-sdk.wit",
    });
}

use bindings::workspace::plugins::host_services::Host as HostServicesHost;
use bindings::DeveloperWorkspacePlugin;

/// Fuel budget per guest call (`docs/04-extensions/plugin-lifecycle.md`
/// §3.1). Re-armed before every `initialize`/`on-event`/`shutdown`
/// invocation, not shared across calls -- a plugin that behaves for 999
/// events and then loops forever on the 1000th must still trap on that one
/// call, not run out of a cumulative budget early.
const FUEL_PER_CALL: u64 = 1_000_000;

/// Linear memory ceiling per plugin instance (`plugin-lifecycle.md` §3.2).
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// Permission Manager checking sandbox operations capabilities.
pub struct PermissionManager;

impl PermissionManager {
    /// Create new manager context.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Assert `requested` is present in `granted` -- exact match. A real
    /// check as of `step16.md` (`PresenceRead` is the first capability this
    /// enforces); the other three `PluginCapability` variants
    /// (`NetworkConnect`/`FsRead`/`FsWrite`/`SlackRead`) are still defined
    /// but unreachable from any host function yet, so nothing calls this
    /// with them. Kept as the single decision point (rather than an inline
    /// `.contains()` at each call site) so a future capability needing more
    /// than exact-match -- e.g. `NetworkConnect` domain-prefix matching --
    /// has one place to grow into.
    #[must_use]
    pub fn verify_capability(
        &self,
        granted: &[PluginCapability],
        requested: &PluginCapability,
    ) -> bool {
        granted.contains(requested)
    }
}

impl Default for PermissionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Supplies the live data behind `get-member-presence` (`step16.md`).
/// Implemented in `crates/app` (the only place that actually holds the
/// `SharedReadModel`) and injected into [`PluginHostManager`], so this
/// crate stays decoupled from `crates/storage`/`crates/commands` -- it
/// already depends on `domain` for `PresenceStatus`, nothing more.
#[async_trait]
pub trait PluginPresenceProvider: Send + Sync {
    /// Look up `user_id`'s current presence, if known.
    async fn presence(&self, user_id: &str) -> Option<domain::PresenceStatus>;
}

/// Where to discover plugin components and which of them are allowed to
/// load (`docs/05-operations/configuration.md`'s `[plugins]` table).
#[derive(Debug, Clone)]
pub struct PluginHostConfig {
    /// Directory scanned for `<plugin_id>.wasm` Component-Model binaries.
    pub directory: PathBuf,
    /// Only plugin ids in this list are loaded, even if present on disk --
    /// mirrors every integration's explicit-opt-in default.
    pub allowed_list: Vec<String>,
}

/// Per-plugin-instance state handed to its WASM `Store` -- carries what the
/// `host-services` `Host` impl below needs (which plugin is calling, where
/// to publish events, what it's allowed to do, and where to get presence
/// data) plus wasmtime's own memory limiter state.
struct PluginState {
    plugin_id: String,
    event_bus: Arc<dyn EventBus>,
    permission_manager: Arc<PermissionManager>,
    granted_capabilities: Vec<PluginCapability>,
    presence_provider: Arc<dyn PluginPresenceProvider>,
    limits: StoreLimits,
}

impl ResourceLimiter for PluginState {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.limits.memory_growing(current, desired, maximum)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.limits.table_growing(current, desired, maximum)
    }
}

impl HostServicesHost for PluginState {
    fn publish_event(
        &mut self,
        event_type: String,
        payload: String,
    ) -> std::result::Result<(), String> {
        // `publish-event` is a synchronous WIT/Component-Model call (this
        // phase's Decision 3 scope has no async host functions), but
        // `EventBus::publish` is async. `Handle::current().block_on` is
        // safe here because every guest call in this crate runs inside a
        // `spawn_blocking` closure (see `load_one`/`deliver_event` below),
        // never directly on a Tokio worker thread. `InProcessEventBus`
        // (`crates/events`) never actually awaits I/O inside `publish` --
        // it's a non-blocking `broadcast::Sender::send` -- so this never
        // stalls waiting on something slow.
        let payload_json = serde_json::json!({
            "event_type": event_type,
            "payload": payload,
        })
        .to_string();
        let event = Event::PluginCustomEvent {
            plugin_id: self.plugin_id.clone(),
            payload_json,
        };
        tokio::runtime::Handle::current()
            .block_on(self.event_bus.publish(event))
            .map_err(|e| e.to_string())
    }

    fn log(&mut self, level: String, message: String) {
        match level.as_str() {
            "trace" => tracing::trace!(plugin_id = %self.plugin_id, "{message}"),
            "debug" => tracing::debug!(plugin_id = %self.plugin_id, "{message}"),
            "warn" => tracing::warn!(plugin_id = %self.plugin_id, "{message}"),
            "error" => tracing::error!(plugin_id = %self.plugin_id, "{message}"),
            // "info" and any unrecognized level both fall back to info,
            // rather than silently dropping a guest's log call over a typo.
            _ => tracing::info!(plugin_id = %self.plugin_id, "{message}"),
        }
    }

    fn get_member_presence(
        &mut self,
        user_id: String,
    ) -> std::result::Result<bindings::workspace::plugins::host_services::PresenceStatus, String>
    {
        if !self
            .permission_manager
            .verify_capability(&self.granted_capabilities, &PluginCapability::PresenceRead)
        {
            return Err("capability not granted: presence-read".to_string());
        }
        let presence =
            tokio::runtime::Handle::current().block_on(self.presence_provider.presence(&user_id));
        presence
            .map(map_presence_status)
            .ok_or_else(|| format!("no presence found for user: {user_id}"))
    }
}

fn map_presence_status(
    status: domain::PresenceStatus,
) -> bindings::workspace::plugins::host_services::PresenceStatus {
    use bindings::workspace::plugins::host_services::PresenceStatus as Wit;
    match status {
        domain::PresenceStatus::Active => Wit::Active,
        domain::PresenceStatus::Away => Wit::Away,
        domain::PresenceStatus::Offline => Wit::Offline,
        domain::PresenceStatus::Meeting => Wit::Meeting,
        domain::PresenceStatus::Lunch => Wit::Lunch,
    }
}

/// A successfully loaded and initialized plugin instance.
struct LoadedPlugin {
    store: Store<PluginState>,
    instance: DeveloperWorkspacePlugin,
}

/// Orchestrator managing compile loops and lifecycles of guest plugins.
pub struct PluginHostManager {
    engine: Engine,
    linker: Linker<PluginState>,
    config: PluginHostConfig,
    event_bus: Arc<dyn EventBus>,
    permission_manager: Arc<PermissionManager>,
    presence_provider: Arc<dyn PluginPresenceProvider>,
    plugins: Mutex<HashMap<String, LoadedPlugin>>,
}

impl PluginHostManager {
    /// Create host coordinator: builds the `wasmtime::Engine` (fuel
    /// metering enabled) and `Linker` (host-services wired in), but loads
    /// nothing yet -- see [`Self::load_all`].
    pub fn new(
        config: PluginHostConfig,
        event_bus: Arc<dyn EventBus>,
        permission_manager: PermissionManager,
        presence_provider: Arc<dyn PluginPresenceProvider>,
    ) -> Result<Self> {
        let mut engine_config = Config::new();
        engine_config.consume_fuel(true);
        let engine = Engine::new(&engine_config)
            .map_err(|e| WorkspaceError::Plugin(format!("engine init failed: {e}")))?;

        let mut linker = Linker::new(&engine);
        DeveloperWorkspacePlugin::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|e| WorkspaceError::Plugin(format!("linker wiring failed: {e}")))?;

        Ok(Self {
            engine,
            linker,
            config,
            event_bus,
            permission_manager: Arc::new(permission_manager),
            presence_provider,
            plugins: Mutex::new(HashMap::new()),
        })
    }

    /// Initialize host context.
    pub fn initialize(&self) -> Result<()> {
        tracing::info!("WASM Plugin Host engine initialized.");
        Ok(())
    }

    /// Discover allow-listed `*.wasm` components under `config.directory`,
    /// compile, instantiate, and call each one's `initialize` export. A
    /// missing directory yields zero plugins (not an error -- the default
    /// `[plugins]` config points nowhere real yet, mirroring every
    /// integration's "not configured" case). An individual plugin's
    /// compile/instantiate/trap failure is logged and only that plugin is
    /// skipped -- one broken plugin must never block every other plugin
    /// from loading (ADR-0002).
    pub async fn load_all(&self) -> Result<()> {
        for (plugin_id, path) in
            discover_plugin_paths(&self.config.directory, &self.config.allowed_list)
        {
            if let Err(e) = self.load_one(&plugin_id, &path).await {
                tracing::error!(plugin_id = %plugin_id, error = %e, "Failed to load plugin; skipping");
            }
        }
        Ok(())
    }

    async fn load_one(&self, plugin_id: &str, path: &Path) -> Result<()> {
        let engine = self.engine.clone();
        let linker = self.linker.clone();
        let path = path.to_path_buf();
        let plugin_id_owned = plugin_id.to_string();
        let event_bus = Arc::clone(&self.event_bus);
        let permission_manager = Arc::clone(&self.permission_manager);
        let presence_provider = Arc::clone(&self.presence_provider);

        let (store, instance) = tokio::task::spawn_blocking(move || {
            load_and_initialize(
                &engine,
                &linker,
                &path,
                plugin_id_owned,
                event_bus,
                permission_manager,
                presence_provider,
            )
        })
        .await
        .map_err(|e| WorkspaceError::Plugin(format!("plugin load task panicked: {e}")))??;

        self.plugins
            .lock()
            .await
            .insert(plugin_id.to_string(), LoadedPlugin { store, instance });
        tracing::info!(plugin_id = %plugin_id, "Plugin loaded and initialized.");
        Ok(())
    }

    /// Call `shutdown` on every loaded plugin and drop the instances.
    /// Called once at workspace exit (`plugin-lifecycle.md`'s
    /// `Active -> Terminated -> Unloaded` transition).
    ///
    /// Runs each guest call inside `spawn_blocking` (see
    /// [`EventHandler::handle`]'s doc comment for why this matters -- same
    /// reasoning applies here).
    pub async fn shutdown_all(&self) {
        let ids: Vec<String> = self.plugins.lock().await.keys().cloned().collect();
        for id in ids {
            let Some(mut loaded) = self.plugins.lock().await.remove(&id) else {
                continue;
            };
            let result = tokio::task::spawn_blocking(move || {
                if loaded.store.set_fuel(FUEL_PER_CALL).is_err() {
                    return Err("fuel re-arm failed".to_string());
                }
                match loaded.instance.call_shutdown(&mut loaded.store) {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(guest_err)) => Err(guest_err),
                    Err(trap) => Err(trap.to_string()),
                }
            })
            .await;
            match result {
                Ok(Ok(())) => tracing::info!(plugin_id = %id, "Plugin shut down cleanly."),
                Ok(Err(e)) => {
                    tracing::warn!(plugin_id = %id, error = %e, "Plugin shutdown returned an error");
                }
                Err(join_err) => {
                    tracing::error!(plugin_id = %id, error = %join_err, "Plugin shutdown task panicked");
                }
            }
        }
    }

    /// Plugin ids currently loaded and initialized (for diagnostics/tests).
    pub async fn loaded_plugin_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.plugins.lock().await.keys().cloned().collect();
        ids.sort();
        ids
    }
}

fn load_and_initialize(
    engine: &Engine,
    linker: &Linker<PluginState>,
    path: &Path,
    plugin_id: String,
    event_bus: Arc<dyn EventBus>,
    permission_manager: Arc<PermissionManager>,
    presence_provider: Arc<dyn PluginPresenceProvider>,
) -> Result<(Store<PluginState>, DeveloperWorkspacePlugin)> {
    let component = Component::from_file(engine, path)
        .map_err(|e| WorkspaceError::Plugin(format!("compile {}: {e}", path.display())))?;

    let granted_capabilities = load_granted_capabilities(path, &plugin_id);

    let limits = StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT_BYTES)
        .build();
    let mut store = Store::new(
        engine,
        PluginState {
            plugin_id: plugin_id.clone(),
            event_bus,
            permission_manager,
            granted_capabilities,
            presence_provider,
            limits,
        },
    );
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(FUEL_PER_CALL)
        .map_err(|e| WorkspaceError::Plugin(format!("fuel setup for {plugin_id}: {e}")))?;

    let instance = DeveloperWorkspacePlugin::instantiate(&mut store, &component, linker)
        .map_err(|e| WorkspaceError::Plugin(format!("instantiate {plugin_id}: {e}")))?;

    instance
        .call_initialize(&mut store, "")
        .map_err(|e| WorkspaceError::Plugin(format!("{plugin_id} initialize trapped: {e}")))?
        .map_err(|e| WorkspaceError::Plugin(format!("{plugin_id} initialize failed: {e}")))?;

    Ok((store, instance))
}

/// The `<plugin-id>.toml` manifest sitting next to a plugin's `.wasm`
/// (`step16.md` Decision 2) -- just the capability list for now, not the
/// fuller `[plugin]`/`[capabilities.network]`/etc. shape
/// `docs/04-extensions/capability-system.md` sketches (more surface than
/// this phase's one real capability needs).
#[derive(Debug, Deserialize)]
struct PluginManifestFile {
    #[serde(default)]
    capabilities: Vec<String>,
}

/// Read `<wasm_path with .toml extension>` and resolve it to a list of
/// granted [`PluginCapability`]s. A **missing** file grants nothing
/// (secure default -- `capability-system.md` §1's "zero-privilege sandbox
/// by default," made real for the first time this phase). A file that
/// exists but fails to parse, or names a capability this host doesn't
/// recognize, is logged and that entry (or the whole file, if
/// unparseable) grants nothing -- fail-closed, but never load-blocking.
fn load_granted_capabilities(wasm_path: &Path, plugin_id: &str) -> Vec<PluginCapability> {
    let manifest_path = wasm_path.with_extension("toml");
    let Ok(contents) = std::fs::read_to_string(&manifest_path) else {
        return Vec::new();
    };

    let manifest: PluginManifestFile = match toml::from_str(&contents) {
        Ok(manifest) => manifest,
        Err(e) => {
            tracing::error!(
                plugin_id = %plugin_id,
                error = %e,
                "Malformed plugin manifest; granting zero capabilities"
            );
            return Vec::new();
        }
    };

    manifest
        .capabilities
        .into_iter()
        .filter_map(|name| match name.as_str() {
            "presence-read" => Some(PluginCapability::PresenceRead),
            other => {
                tracing::warn!(
                    plugin_id = %plugin_id,
                    capability = %other,
                    "Unknown capability in manifest; ignoring"
                );
                None
            }
        })
        .collect()
}

/// Scan `directory` (non-recursive) for `<id>.wasm` files whose stem
/// appears in `allowed_list`, returning `(plugin_id, path)` pairs sorted by
/// id for deterministic load order. A missing/unreadable directory yields
/// an empty list rather than an error -- see [`PluginHostManager::load_all`].
fn discover_plugin_paths(directory: &Path, allowed_list: &[String]) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };

    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("wasm"))
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?.to_string();
            allowed_list.contains(&stem).then_some((stem, path))
        })
        .collect();

    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Outcome of one guest `on-event`/`shutdown` call, computed inside a
/// `spawn_blocking` closure (see [`EventHandler::handle`]) and reported
/// back out alongside the [`LoadedPlugin`] the closure took ownership of.
enum CallOutcome {
    Ok,
    GuestError(String),
    Trapped(String),
}

#[async_trait]
impl EventHandler for PluginHostManager {
    /// Forward every `Event` published on the shared bus into each loaded
    /// plugin's `on-event` export, JSON-serialized
    /// (`docs/02-architecture/events.md`). A trap (fuel exhaustion, OOM, a
    /// genuine guest bug) is caught, logged, and that plugin instance is
    /// dropped -- it does not take down the host process or block delivery
    /// to any other plugin (ADR-0002).
    ///
    /// Each guest call runs inside `tokio::task::spawn_blocking`, not
    /// directly on whatever worker thread is running this async fn. Two
    /// reasons: wasmtime execution (even fuel-limited) is synchronous
    /// CPU-bound work that shouldn't occupy an async executor's worker
    /// thread, and -- found the hard way, `step16.md` Implementation Notes
    /// -- host functions that need to call back into async code
    /// (`publish-event`, `get-member-presence`) use
    /// `tokio::runtime::Handle::current().block_on(..)`, which panics with
    /// "Cannot start a runtime from within a runtime" if invoked directly
    /// on a worker thread instead of a blocking-pool thread. `load_one`
    /// already ran guest calls this way; this brings `on-event`/`shutdown`
    /// into line with the invariant `PluginState::publish_event`'s own doc
    /// comment already claimed but which wasn't actually true here until
    /// this fix.
    async fn handle(&self, event: Event) -> Result<()> {
        let event_type = event_type_name(&event).to_string();
        let payload = serde_json::to_string(&event)
            .map_err(|e| WorkspaceError::Plugin(format!("event serialization failed: {e}")))?;

        let ids: Vec<String> = self.plugins.lock().await.keys().cloned().collect();

        for id in ids {
            let Some(mut loaded) = self.plugins.lock().await.remove(&id) else {
                continue;
            };

            let event_type = event_type.clone();
            let payload = payload.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                let outcome = if loaded.store.set_fuel(FUEL_PER_CALL).is_err() {
                    CallOutcome::Trapped("fuel re-arm failed".to_string())
                } else {
                    match loaded
                        .instance
                        .call_on_event(&mut loaded.store, &event_type, &payload)
                    {
                        Ok(Ok(())) => CallOutcome::Ok,
                        Ok(Err(guest_err)) => CallOutcome::GuestError(guest_err),
                        Err(trap) => CallOutcome::Trapped(trap.to_string()),
                    }
                };
                (loaded, outcome)
            })
            .await;

            match outcome {
                Ok((loaded, CallOutcome::Ok)) => {
                    self.plugins.lock().await.insert(id, loaded);
                }
                Ok((loaded, CallOutcome::GuestError(guest_err))) => {
                    tracing::warn!(plugin_id = %id, error = %guest_err, "Plugin on-event returned an error");
                    self.plugins.lock().await.insert(id, loaded);
                }
                Ok((_loaded, CallOutcome::Trapped(trap))) => {
                    tracing::error!(plugin_id = %id, error = %trap, "Plugin trapped handling event; suspending");
                }
                Err(join_err) => {
                    tracing::error!(plugin_id = %id, error = %join_err, "Plugin on-event task panicked; suspending");
                }
            }
        }

        Ok(())
    }
}

/// Mirrors `crates/events`' own private `event_type_name` -- that one isn't
/// `pub`, and this crate needs the same string for the `event-type`
/// parameter of `on-event` (`docs/02-architecture/events.md`'s "all events
/// are serialized to JSON before... dispatch to WASM plugins" contract).
fn event_type_name(event: &Event) -> &'static str {
    match event {
        Event::SlackMessageReceived(_) => "SlackMessageReceived",
        Event::SlackPresenceChanged(_) => "SlackPresenceChanged",
        Event::GitHubPRCreated(_) => "GitHubPRCreated",
        Event::CalendarReminderTriggered(_) => "CalendarReminderTriggered",
        Event::SystemAlert(_) => "SystemAlert",
        Event::PluginCustomEvent { .. } => "PluginCustomEvent",
        Event::IntegrationStatusChanged { .. } => "IntegrationStatusChanged",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `PluginPresenceProvider` for tests: returns whatever's in
    /// `responses` and records every `user_id` it was actually called
    /// with, so a test can assert enforcement happened *before* reaching
    /// the provider (Decision 5, `step16.md`) rather than only that the
    /// response differed.
    #[derive(Default)]
    struct MockPresenceProvider {
        responses: HashMap<String, domain::PresenceStatus>,
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl PluginPresenceProvider for MockPresenceProvider {
        async fn presence(&self, user_id: &str) -> Option<domain::PresenceStatus> {
            self.calls.lock().await.push(user_id.to_string());
            self.responses.get(user_id).copied()
        }
    }

    fn no_presence_provider() -> Arc<dyn PluginPresenceProvider> {
        Arc::new(MockPresenceProvider::default())
    }

    #[test]
    fn verify_capability_grants_exact_matches_only() {
        let pm = PermissionManager::new();
        let granted = vec![PluginCapability::PresenceRead];
        assert!(pm.verify_capability(&granted, &PluginCapability::PresenceRead));
        assert!(!pm.verify_capability(&granted, &PluginCapability::SlackRead));
        assert!(!pm.verify_capability(&[], &PluginCapability::PresenceRead));
    }

    #[test]
    fn load_granted_capabilities_with_no_manifest_file_grants_nothing() {
        let dir = std::env::temp_dir().join(format!("tw_manifest_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let wasm_path = dir.join("hello.wasm");
        std::fs::write(&wasm_path, b"").unwrap();

        let granted = load_granted_capabilities(&wasm_path, "hello");

        assert!(granted.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_granted_capabilities_parses_a_real_manifest() {
        let dir = std::env::temp_dir().join(format!("tw_manifest_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let wasm_path = dir.join("presence-checker.wasm");
        std::fs::write(&wasm_path, b"").unwrap();
        std::fs::write(
            dir.join("presence-checker.toml"),
            "capabilities = [\"presence-read\"]\n",
        )
        .unwrap();

        let granted = load_granted_capabilities(&wasm_path, "presence-checker");

        assert_eq!(granted, vec![PluginCapability::PresenceRead]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_granted_capabilities_on_a_malformed_manifest_grants_nothing_not_panics() {
        let dir = std::env::temp_dir().join(format!("tw_manifest_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let wasm_path = dir.join("hello.wasm");
        std::fs::write(&wasm_path, b"").unwrap();
        std::fs::write(dir.join("hello.toml"), "not valid = = toml").unwrap();

        let granted = load_granted_capabilities(&wasm_path, "hello");

        assert!(granted.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_granted_capabilities_ignores_unknown_capability_names() {
        let dir = std::env::temp_dir().join(format!("tw_manifest_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let wasm_path = dir.join("hello.wasm");
        std::fs::write(&wasm_path, b"").unwrap();
        std::fs::write(
            dir.join("hello.toml"),
            "capabilities = [\"time-travel\", \"presence-read\"]\n",
        )
        .unwrap();

        let granted = load_granted_capabilities(&wasm_path, "hello");

        assert_eq!(granted, vec![PluginCapability::PresenceRead]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_plugin_paths_filters_by_allow_list_and_extension() {
        let dir =
            std::env::temp_dir().join(format!("tw_plugin_host_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.wasm"), b"").unwrap();
        std::fs::write(dir.join("not-allowed.wasm"), b"").unwrap();
        std::fs::write(dir.join("hello.txt"), b"").unwrap();

        let found = discover_plugin_paths(&dir, &["hello".to_string()]);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "hello");
        assert_eq!(found[0].1, dir.join("hello.wasm"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_plugin_paths_on_missing_directory_returns_empty_not_error() {
        let dir =
            std::env::temp_dir().join(format!("tw_plugin_host_missing_{}", uuid::Uuid::new_v4()));
        let found = discover_plugin_paths(&dir, &["hello".to_string()]);
        assert!(found.is_empty());
    }

    #[test]
    fn discover_plugin_paths_sorts_results_deterministically() {
        let dir =
            std::env::temp_dir().join(format!("tw_plugin_host_sort_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zeta.wasm"), b"").unwrap();
        std::fs::write(dir.join("alpha.wasm"), b"").unwrap();

        let found = discover_plugin_paths(&dir, &["zeta".to_string(), "alpha".to_string()]);

        assert_eq!(found[0].0, "alpha");
        assert_eq!(found[1].0, "zeta");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn load_all_on_empty_directory_yields_no_plugins() {
        let dir =
            std::env::temp_dir().join(format!("tw_plugin_host_empty_{}", uuid::Uuid::new_v4()));
        let event_bus = Arc::new(events::InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let manager = PluginHostManager::new(
            PluginHostConfig {
                directory: dir,
                allowed_list: vec!["hello".to_string()],
            },
            event_bus,
            PermissionManager::new(),
            no_presence_provider(),
        )
        .expect("manager construction must succeed");

        manager.load_all().await.expect("load_all must not error");

        assert!(manager.loaded_plugin_ids().await.is_empty());
    }

    #[tokio::test]
    async fn load_one_on_a_garbage_wasm_file_is_skipped_not_fatal() {
        let dir =
            std::env::temp_dir().join(format!("tw_plugin_host_garbage_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.wasm"), b"not a real component").unwrap();

        let event_bus = Arc::new(events::InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let manager = PluginHostManager::new(
            PluginHostConfig {
                directory: dir.clone(),
                allowed_list: vec!["hello".to_string()],
            },
            event_bus,
            PermissionManager::new(),
            no_presence_provider(),
        )
        .expect("manager construction must succeed");

        manager.load_all().await.expect("load_all must not error");

        assert!(manager.loaded_plugin_ids().await.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `examples/plugins/<name>/target/wasm32-wasip1/debug/<name>.wasm`,
    /// built by `cargo component build` (`step14.md` Decision 2).
    /// `cargo-component` is a plugin-author-side tool, not a standard
    /// contributor requirement (`step14.md`'s Context), so it isn't run as
    /// part of `cargo test --workspace` -- these tests skip (with a clear
    /// message, not a silent no-op) rather than hard-fail when the `.wasm`
    /// hasn't been built in this environment.
    fn example_plugin_wasm_path(name: &str) -> PathBuf {
        // `wasm32-unknown-unknown`, not `wasm32-wasip1`: our WIT world
        // imports only our own `host-services`, no `wasi:*` interfaces, so
        // targeting WASI Preview 1 pulls in a WASI adapter that expects
        // `wasi:cli/environment` etc. wired into the host `Linker` --
        // confirmed by a real "component imports instance
        // `wasi:cli/environment@0.2.3`... not found" trap the first time
        // this was tried. `wasm32-unknown-unknown` produces a component
        // whose only imports are the ones this host actually provides,
        // which is also the more correct sandboxing default for a plugin
        // that shouldn't have ambient env/clock/fs access in the first
        // place (`step14.md` Decision 3's minimal host-function scope).
        // Rust converts `-` to `_` in library artifact file names, but not
        // in the crate/directory name itself -- `examples/plugins/
        // presence-checker/` builds `presence_checker.wasm`, so the two
        // can't share one literal string for hyphenated plugin names.
        let wasm_file_stem = name.replace('-', "_");
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("plugins")
            .join(name)
            .join("target")
            .join("wasm32-unknown-unknown")
            .join("debug")
            .join(format!("{wasm_file_stem}.wasm"))
    }

    macro_rules! require_example_plugin {
        ($name:expr) => {{
            let path = example_plugin_wasm_path($name);
            if !path.exists() {
                eprintln!(
                    "skipping: {} not built -- run `cargo component build` in \
                     examples/plugins/{} first",
                    path.display(),
                    $name
                );
                return;
            }
            path
        }};
    }

    #[tokio::test]
    async fn load_all_loads_and_initializes_the_real_hello_plugin() {
        let _ = tracing_subscriber::fmt::try_init();
        let wasm_path = require_example_plugin!("hello");

        let dir =
            std::env::temp_dir().join(format!("tw_plugin_host_hello_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&wasm_path, dir.join("hello.wasm")).unwrap();

        let event_bus = Arc::new(events::InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let manager = PluginHostManager::new(
            PluginHostConfig {
                directory: dir.clone(),
                allowed_list: vec!["hello".to_string()],
            },
            event_bus,
            PermissionManager::new(),
            no_presence_provider(),
        )
        .expect("manager construction must succeed");

        manager
            .load_all()
            .await
            .expect("load_all of a real component must not error");
        assert_eq!(manager.loaded_plugin_ids().await, vec!["hello".to_string()]);

        manager
            .handle(Event::SystemAlert("test".into()))
            .await
            .expect("handle must not error");
        // on-event completed without trapping -- the plugin is still loaded.
        assert_eq!(manager.loaded_plugin_ids().await, vec!["hello".to_string()]);

        manager.shutdown_all().await;
        assert!(manager.loaded_plugin_ids().await.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_fuel_exhausting_guest_traps_and_is_suspended_not_hanging_the_host() {
        let wasm_path = require_example_plugin!("looper");

        let dir =
            std::env::temp_dir().join(format!("tw_plugin_host_looper_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&wasm_path, dir.join("looper.wasm")).unwrap();

        let event_bus = Arc::new(events::InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let manager = PluginHostManager::new(
            PluginHostConfig {
                directory: dir.clone(),
                allowed_list: vec!["looper".to_string()],
            },
            event_bus,
            PermissionManager::new(),
            no_presence_provider(),
        )
        .expect("manager construction must succeed");

        manager
            .load_all()
            .await
            .expect("load_all of a real component must not error");
        assert_eq!(
            manager.loaded_plugin_ids().await,
            vec!["looper".to_string()]
        );

        // The critical assertion (`step14.md` Verification Plan): an
        // infinite loop in `on-event` must trap on the fuel budget, not
        // hang this test (and, in production, the host process).
        manager
            .handle(Event::SystemAlert("test".into()))
            .await
            .expect("handle must not error even though the plugin traps internally");

        // The trapped instance was suspended (dropped), proving the trap
        // was actually caught rather than propagated/ignored.
        assert!(manager.loaded_plugin_ids().await.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_memory_hogging_guest_traps_and_is_suspended_not_growing_unbounded() {
        let wasm_path = require_example_plugin!("memhog");

        let dir =
            std::env::temp_dir().join(format!("tw_plugin_host_memhog_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&wasm_path, dir.join("memhog.wasm")).unwrap();

        let event_bus = Arc::new(events::InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let manager = PluginHostManager::new(
            PluginHostConfig {
                directory: dir.clone(),
                allowed_list: vec!["memhog".to_string()],
            },
            event_bus,
            PermissionManager::new(),
            no_presence_provider(),
        )
        .expect("manager construction must succeed");

        manager
            .load_all()
            .await
            .expect("load_all of a real component must not error");
        assert_eq!(
            manager.loaded_plugin_ids().await,
            vec!["memhog".to_string()]
        );

        // The critical assertion (`step14.md` Verification Plan): an
        // allocation far past the 64MB per-instance ceiling must trap
        // (`MEMORY_LIMIT_BYTES`, `ResourceLimiter::memory_growing`), not
        // let the guest grow the host process's memory unbounded.
        manager
            .handle(Event::SystemAlert("test".into()))
            .await
            .expect("handle must not error even though the plugin traps internally");

        assert!(manager.loaded_plugin_ids().await.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn get_member_presence_reaches_the_provider_when_the_capability_is_granted() {
        let wasm_path = require_example_plugin!("presence-checker");

        let dir = std::env::temp_dir().join(format!(
            "tw_plugin_host_presence_granted_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&wasm_path, dir.join("presence_checker.wasm")).unwrap();
        std::fs::write(
            dir.join("presence_checker.toml"),
            "capabilities = [\"presence-read\"]\n",
        )
        .unwrap();

        let event_bus = Arc::new(events::InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let mut responses = HashMap::new();
        responses.insert("local-user".to_string(), domain::PresenceStatus::Active);
        let presence_provider = Arc::new(MockPresenceProvider {
            responses,
            calls: Mutex::new(Vec::new()),
        });

        let manager = PluginHostManager::new(
            PluginHostConfig {
                directory: dir.clone(),
                allowed_list: vec!["presence_checker".to_string()],
            },
            event_bus,
            PermissionManager::new(),
            Arc::clone(&presence_provider) as Arc<dyn PluginPresenceProvider>,
        )
        .expect("manager construction must succeed");

        manager
            .load_all()
            .await
            .expect("load_all of a real component must not error");
        assert_eq!(
            manager.loaded_plugin_ids().await,
            vec!["presence_checker".to_string()]
        );

        manager
            .handle(Event::SystemAlert("test".into()))
            .await
            .expect("handle must not error");

        // The critical assertion (`step16.md` Decision 5): the provider
        // WAS reached, proving the granted capability actually let the
        // call through, not just that some response came back.
        assert_eq!(
            presence_provider.calls.lock().await.as_slice(),
            ["local-user".to_string()]
        );
        // No trap -- the plugin is still loaded.
        assert_eq!(
            manager.loaded_plugin_ids().await,
            vec!["presence_checker".to_string()]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn get_member_presence_is_denied_before_reaching_the_provider_without_the_capability() {
        let wasm_path = require_example_plugin!("presence-checker");

        let dir = std::env::temp_dir().join(format!(
            "tw_plugin_host_presence_denied_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&wasm_path, dir.join("presence_checker.wasm")).unwrap();
        // Deliberately no presence_checker.toml manifest -- secure default
        // (`step16.md` Decision 2).

        let event_bus = Arc::new(events::InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let mut responses = HashMap::new();
        responses.insert("local-user".to_string(), domain::PresenceStatus::Active);
        let presence_provider = Arc::new(MockPresenceProvider {
            responses,
            calls: Mutex::new(Vec::new()),
        });

        let manager = PluginHostManager::new(
            PluginHostConfig {
                directory: dir.clone(),
                allowed_list: vec!["presence_checker".to_string()],
            },
            event_bus,
            PermissionManager::new(),
            Arc::clone(&presence_provider) as Arc<dyn PluginPresenceProvider>,
        )
        .expect("manager construction must succeed");

        manager
            .load_all()
            .await
            .expect("load_all of a real component must not error");

        manager
            .handle(Event::SystemAlert("test".into()))
            .await
            .expect("handle must not error");

        // The critical assertion (`step16.md` Decision 5): the provider was
        // NEVER called -- proving denial happens before reaching it, not
        // just that the guest happened to get a different response.
        assert!(presence_provider.calls.lock().await.is_empty());
        // No trap -- a denial is a normal `Err` return from
        // get-member-presence, not a guest panic, so the plugin stays
        // loaded.
        assert_eq!(
            manager.loaded_plugin_ids().await,
            vec!["presence_checker".to_string()]
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
