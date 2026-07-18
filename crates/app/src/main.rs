//! Main entry point for the Terminal-first Developer Workspace.

use async_trait::async_trait;
use commands::{
    Command, CommandDispatcher, InMemoryCommandDispatcher, Projector, SelectionApplier,
    SlackSelectionApplier, WorkspaceCommandHandler,
};
use common::Result;
use config::AppConfig;
use domain::{
    FailedEventRepository, IntegrationSource, NotificationRepository, PresenceRepository,
    PresenceStatus,
};
use events::{
    EventBus, EventDispatcher, EventHandler, InProcessEventBus, IntegrationConnectionStatus,
};
use integration::{
    CalendarAdapter, CalendarConfig, ConnectionStatus, GitHubAdapter, GitHubConfig,
    IntegrationAdapter, IntegrationConnector, Picker, SlackAdapter, SlackConfig, SlackMessenger,
    SlackPicker,
};
use logging::{init_logger, spans::application_span};
use plugin_host::{PermissionManager, PluginHostConfig, PluginHostManager};
use registry::InMemoryUiRegistry;
use secrets::{SecretProviderChain, SecretWriter};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use storage::RedbStorageBackend;
use tokio::sync::Mutex;
use tracing::info;
use ui::TuiRenderer;

/// Composition-root glue for `Command::ApplySlackSelection` (`step8.md`):
/// combines writing `config.toml` (a Config/Workspace concern) with
/// restarting the Slack adapter's poll loop (an Integration concern).
/// Neither `crates/config` nor `crates/integration` should know about the
/// other, so this bridging type lives here instead of in either crate —
/// see `SlackSelectionApplier`'s doc comment in `crates/commands` for why
/// it isn't just a method on `SlackAdapter`.
struct ConfigFileSlackSelectionApplier {
    slack_adapter: Arc<SlackAdapter>,
    config_path: PathBuf,
    base_config: Mutex<AppConfig>,
}

#[async_trait]
impl SlackSelectionApplier for ConfigFileSlackSelectionApplier {
    async fn apply(
        &self,
        event_bus: Arc<dyn EventBus>,
        channel_ids: Vec<String>,
        watched_user_ids: Vec<String>,
    ) -> Result<()> {
        {
            let mut cfg = self.base_config.lock().await;
            cfg.integrations.slack.channel_ids = channel_ids.clone();
            cfg.integrations.slack.watched_user_ids = watched_user_ids.clone();
            cfg.integrations.slack.enabled = true;
            cfg.save_to(&self.config_path)?;
        }
        self.slack_adapter
            .update_selection(event_bus, channel_ids, watched_user_ids)
            .await
    }
}

/// GitHub's equivalent of [`ConfigFileSlackSelectionApplier`] (`step10.md`),
/// same cross-context bridging reasoning. Implements the generic
/// [`SelectionApplier`] (`step11.md`) rather than a bespoke
/// `GitHubSelectionApplier` — GitHub's single-list selection is exactly the
/// shape that trait generalizes.
struct ConfigFileGitHubSelectionApplier {
    github_adapter: Arc<GitHubAdapter>,
    config_path: PathBuf,
    base_config: Mutex<AppConfig>,
}

#[async_trait]
impl SelectionApplier for ConfigFileGitHubSelectionApplier {
    async fn apply(&self, event_bus: Arc<dyn EventBus>, items: Vec<String>) -> Result<()> {
        {
            let mut cfg = self.base_config.lock().await;
            cfg.integrations.github.repositories = items.clone();
            cfg.integrations.github.enabled = true;
            cfg.save_to(&self.config_path)?;
        }
        self.github_adapter.update_selection(event_bus, items).await
    }
}

/// Maps `integration::ConnectionStatus` to the structurally-identical but
/// separately-defined `events::IntegrationConnectionStatus` (ADR-0016) —
/// used once at boot to seed `TuiRenderer`'s initial header status before
/// anything's been published to the event bus yet.
fn to_event_status(status: ConnectionStatus) -> IntegrationConnectionStatus {
    match status {
        ConnectionStatus::Disconnected => IntegrationConnectionStatus::Disconnected,
        ConnectionStatus::Connecting => IntegrationConnectionStatus::Connecting,
        ConnectionStatus::Connected => IntegrationConnectionStatus::Connected,
        ConnectionStatus::Reconnecting => IntegrationConnectionStatus::Reconnecting,
        ConnectionStatus::Failed(reason) => IntegrationConnectionStatus::Failed(reason),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Load layered configuration (Default -> config.toml -> Env -> CLI).
    //    Zero Configuration: creates the config directory/file on first run
    //    if none exists yet (see docs/05-operations/configuration.md §4).
    let config = AppConfig::load_or_create_default()?;

    // 2. Initialize logging using the resolved log level, then enter the
    //    root Application span (see docs/05-operations/logging.md §0).
    init_logger(&config.core.log_level)?;
    let _application_span = application_span().entered();
    info!("Starting Terminal Workspace Core Daemon...");
    info!("Workspace settings loaded successfully.");

    // 3. Open the redb storage backend. Creates workspace.redb on first run
    //    (see docs/05-operations/storage.md §2 and ADR-0014).
    let storage = Arc::new(RedbStorageBackend::open(&storage::standard_db_path()).await?);
    info!("Storage backend ready.");

    // 4. Always construct the Slack adapter — the in-app setup overlay
    //    (`step7.md`, `Ctrl+S`) needs something to connect through even
    //    before anything is configured, not just when `enabled = true`.
    //    Resolve whatever credential already exists via SecretProviderChain
    //    (ADR-0006); `initialize()` never errors on a missing token, it
    //    just reports Disconnected (docs/04-extensions/integration-contract.md §2.3).
    //    The chain is also the adapter's write target for a token entered
    //    through that overlay (`SlackAdapter::connect`, `step7.md`).
    let secret_chain = Arc::new(SecretProviderChain::default_chain());
    let slack_adapter = Arc::new(SlackAdapter::new(
        SlackConfig {
            channel_ids: config.integrations.slack.channel_ids.clone(),
            watched_user_ids: config.integrations.slack.watched_user_ids.clone(),
            sync_interval_secs: config.integrations.slack.sync_interval_secs,
        },
        Arc::clone(&secret_chain) as Arc<dyn SecretWriter>,
    ));
    slack_adapter.initialize(secret_chain.as_ref()).await?;
    let slack_messenger = Arc::clone(&slack_adapter) as Arc<dyn SlackMessenger>;
    let slack_connector = Arc::clone(&slack_adapter) as Arc<dyn IntegrationConnector>;
    let slack_picker = Arc::clone(&slack_adapter) as Arc<dyn SlackPicker>;
    // Bridges config.toml persistence + the adapter's live poll loop for
    // Command::ApplySlackSelection (step8.md's channel/user picker,
    // Ctrl+P) — see ConfigFileSlackSelectionApplier's doc comment above.
    let slack_selection_applier: Arc<dyn SlackSelectionApplier> =
        Arc::new(ConfigFileSlackSelectionApplier {
            slack_adapter: Arc::clone(&slack_adapter),
            config_path: config::resolve_config_path(),
            base_config: Mutex::new(config.clone()),
        });

    // 4b. GitHub's equivalent of step 4 (`step10.md`) — always constructed
    //     for the same "the setup overlay needs something to connect
    //     through" reason.
    let github_adapter = Arc::new(GitHubAdapter::new(
        GitHubConfig {
            repositories: config.integrations.github.repositories.clone(),
            sync_interval_secs: config.integrations.github.sync_interval_secs,
        },
        Arc::clone(&secret_chain) as Arc<dyn SecretWriter>,
    ));
    github_adapter.initialize(secret_chain.as_ref()).await?;
    let github_connector = Arc::clone(&github_adapter) as Arc<dyn IntegrationConnector>;
    let github_picker = Arc::clone(&github_adapter) as Arc<dyn Picker>;
    let github_selection_applier: Arc<dyn SelectionApplier> =
        Arc::new(ConfigFileGitHubSelectionApplier {
            github_adapter: Arc::clone(&github_adapter),
            config_path: config::resolve_config_path(),
            base_config: Mutex::new(config.clone()),
        });

    // 4c. Calendar's equivalent of step 4 (`step12.md`) — always
    //     constructed for the same "the setup overlay needs something to
    //     connect through" reason. No messenger, no picker, no selection
    //     applier: Calendar is read-only with no "list my calendars"
    //     discovery call under the secret-URL auth model (`step12.md`
    //     Decision 1's consequence), so there is nothing beyond a
    //     connector to wire up here.
    let calendar_adapter = Arc::new(CalendarAdapter::new(
        CalendarConfig {
            lookahead_hours: config.integrations.calendar.lookahead_hours,
            sync_interval_secs: config.integrations.calendar.sync_interval_secs,
        },
        Arc::clone(&secret_chain) as Arc<dyn SecretWriter>,
    ));
    calendar_adapter.initialize(secret_chain.as_ref()).await?;
    let calendar_connector = Arc::clone(&calendar_adapter) as Arc<dyn IntegrationConnector>;

    // 5. Wire the CQRS write path: Command -> WorkspaceCommandHandler ->
    //    Storage + EventBus (see docs/06-development/decisions/0007-cqrs.md).
    //    Connectors/selection-appliers are keyed registries (`step11.md`)
    //    rather than one named `Option` field per integration — a future
    //    Calendar adds a key to each map instead of growing this call's
    //    argument list again.
    let mut connectors: HashMap<IntegrationSource, Arc<dyn IntegrationConnector>> = HashMap::new();
    connectors.insert(IntegrationSource::Slack, slack_connector);
    connectors.insert(IntegrationSource::GitHub, github_connector);
    connectors.insert(IntegrationSource::Calendar, calendar_connector);
    let mut selection_appliers: HashMap<IntegrationSource, Arc<dyn SelectionApplier>> =
        HashMap::new();
    selection_appliers.insert(IntegrationSource::GitHub, github_selection_applier);

    let event_bus = Arc::new(InProcessEventBus::new(256));
    let handler = Arc::new(WorkspaceCommandHandler::new(
        Arc::clone(&storage) as Arc<dyn PresenceRepository>,
        Arc::clone(&storage) as Arc<dyn NotificationRepository>,
        Arc::clone(&event_bus) as Arc<dyn EventBus>,
        Some(slack_messenger),
        Some(slack_selection_applier),
        connectors,
        selection_appliers,
    ));
    let dispatcher: Arc<dyn CommandDispatcher> = Arc::new(InMemoryCommandDispatcher::new(handler));

    // 6. Auto-start the Slack poll loop at boot if either `enabled = true`
    //    in config.toml or a credential was already found (e.g. connected
    //    through the setup overlay on a previous run) — either signal is
    //    strong enough on its own; requiring both would mean a token saved
    //    through the UI never auto-reconnects on the next launch, which
    //    would defeat the entire point of persisting it (step7.md).
    let already_connected = !matches!(
        slack_adapter.health_check().await?,
        ConnectionStatus::Disconnected
    );
    if config.integrations.slack.enabled || already_connected {
        slack_adapter
            .start(Arc::clone(&event_bus) as Arc<dyn EventBus>)
            .await?;
        info!("Slack adapter started.");
    }

    // 6b. GitHub's equivalent of step 6 (`step10.md`), same reasoning.
    let github_already_connected = !matches!(
        github_adapter.health_check().await?,
        ConnectionStatus::Disconnected
    );
    if config.integrations.github.enabled || github_already_connected {
        github_adapter
            .start(Arc::clone(&event_bus) as Arc<dyn EventBus>)
            .await?;
        info!("GitHub adapter started.");
    }

    // 6c. Calendar's equivalent of step 6 (`step12.md`), same reasoning.
    let calendar_already_connected = !matches!(
        calendar_adapter.health_check().await?,
        ConnectionStatus::Disconnected
    );
    if config.integrations.calendar.enabled || calendar_already_connected {
        calendar_adapter
            .start(Arc::clone(&event_bus) as Arc<dyn EventBus>)
            .await?;
        info!("Calendar adapter started.");
    }

    // 7. Wire the CQRS read path (Phase 5): Projector keeps a
    //    DashboardReadModel current for the TUI to render from — closes
    //    the read path docs/06-development/decisions/0007-cqrs.md deferred
    //    until a real UI consumer existed.
    let presence_repo = Arc::clone(&storage) as Arc<dyn PresenceRepository>;
    let notification_repo = Arc::clone(&storage) as Arc<dyn NotificationRepository>;
    let (projector, read_model) = Projector::new(&presence_repo, &notification_repo).await?;

    // 8. Wire event reliability (retry/backoff + Dead Letter Queue — see
    //    docs/06-development/decisions/0003-event-bus.md's Phase 3
    //    amendment) and register the Projector as a handler.
    let event_dispatcher = EventDispatcher::new(Arc::clone(&event_bus) as Arc<dyn EventBus>)
        .with_dlq(Arc::clone(&storage) as Arc<dyn FailedEventRepository>);
    event_dispatcher
        .register_handler(Arc::new(projector) as Arc<dyn EventHandler>)
        .await;

    // 8b. Plugin runtime (`step14.md`, ADR-0002/0009/0017): default-off,
    //     mirroring every integration's `enabled` toggle (Decision 4) —
    //     nothing under `crates/plugin-host` loads or runs unless a
    //     contributor both flips `[plugins].enabled` and points
    //     `directory`/`allowed_list` at a real plugin. Registered as an
    //     `EventHandler` the same way the Projector is above, so every
    //     `Event` on the shared bus reaches each loaded plugin's
    //     `on-event` export.
    let plugin_host = Arc::new(PluginHostManager::new(
        PluginHostConfig {
            directory: config.plugins.directory.clone(),
            allowed_list: config.plugins.allowed_list.clone(),
        },
        Arc::clone(&event_bus) as Arc<dyn EventBus>,
        PermissionManager::new(),
    )?);
    if config.plugins.enabled {
        plugin_host.initialize()?;
        plugin_host.load_all().await?;
        event_dispatcher
            .register_handler(Arc::clone(&plugin_host) as Arc<dyn EventHandler>)
            .await;
        info!("Plugin host started.");
    }

    event_dispatcher.start();

    // 9. Prove the write path end-to-end with a startup presence command.
    dispatcher
        .dispatch(Command::SetPresence {
            status: PresenceStatus::Active,
            custom_text: None,
        })
        .await?;
    info!("CQRS write path verified: startup SetPresence command dispatched.");

    // 10. Enter the interactive TUI shell (Phase 5) — runs until Ctrl+Q.
    //     Passing `dispatcher` (Phase 7) is what lets the setup overlay's
    //     `Ctrl+S` actually mutate anything — before this the TUI was pure
    //     CQRS read side with no write path of its own. `event_bus` +
    //     the initial status (Phase 9) let the header stay live via a
    //     direct EventBus subscription instead of polling on redraw.
    let initial_slack_status = to_event_status(slack_adapter.health_check().await?);
    let initial_github_status = to_event_status(github_adapter.health_check().await?);
    let initial_calendar_status = to_event_status(calendar_adapter.health_check().await?);
    let ui_registry = Arc::new(InMemoryUiRegistry::new());
    let mut pickers: HashMap<IntegrationSource, Arc<dyn Picker>> = HashMap::new();
    pickers.insert(IntegrationSource::GitHub, github_picker);
    let renderer = TuiRenderer::new(
        ui_registry,
        read_model,
        Arc::clone(&dispatcher),
        slack_picker,
        pickers,
        Arc::clone(&event_bus) as Arc<dyn EventBus>,
        initial_slack_status,
        initial_github_status,
        initial_calendar_status,
    );
    renderer.run_loop().await?;

    // 11. Give every loaded plugin a chance to flush/clean up
    //     (`plugin-lifecycle.md`'s `Active -> Terminated -> Unloaded`
    //     transition) before the process exits. A no-op when the plugin
    //     runtime was never enabled (empty plugin set).
    plugin_host.shutdown_all().await;

    info!("Terminal Workspace exited cleanly.");
    Ok(())
}
