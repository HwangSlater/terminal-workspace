//! Main entry point for the Terminal-first Developer Workspace.

use commands::{
    Command, CommandDispatcher, InMemoryCommandDispatcher, Projector, WorkspaceCommandHandler,
};
use common::Result;
use config::AppConfig;
use domain::{FailedEventRepository, NotificationRepository, PresenceRepository, PresenceStatus};
use events::{EventBus, EventDispatcher, EventHandler, InProcessEventBus};
use integration::{IntegrationAdapter, SlackAdapter, SlackConfig, SlackMessenger};
use logging::{init_logger, spans::application_span};
use registry::InMemoryUiRegistry;
use secrets::SecretProviderChain;
use std::sync::Arc;
use storage::RedbStorageBackend;
use tracing::info;
use ui::TuiRenderer;

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

    // 4. Construct the Slack adapter (Phase 6) if enabled, resolving its
    //    credential via SecretProviderChain (ADR-0006) before anything else
    //    needs it. `initialize()` never errors on a missing token — it just
    //    reports Disconnected (docs/04-extensions/integration-contract.md §2.3).
    let slack_adapter: Option<Arc<SlackAdapter>> = if config.integrations.slack.enabled {
        let adapter = Arc::new(SlackAdapter::new(SlackConfig {
            channel_ids: config.integrations.slack.channel_ids.clone(),
            watched_user_ids: config.integrations.slack.watched_user_ids.clone(),
            sync_interval_secs: config.integrations.slack.sync_interval_secs,
        }));
        let secret_chain = SecretProviderChain::default_chain();
        adapter.initialize(&secret_chain).await?;
        Some(adapter)
    } else {
        None
    };
    let slack_messenger: Option<Arc<dyn SlackMessenger>> = slack_adapter
        .as_ref()
        .map(|adapter| Arc::clone(adapter) as Arc<dyn SlackMessenger>);

    // 5. Wire the CQRS write path: Command -> WorkspaceCommandHandler ->
    //    Storage + EventBus (see docs/06-development/decisions/0007-cqrs.md).
    let event_bus = Arc::new(InProcessEventBus::new(256));
    let handler = Arc::new(WorkspaceCommandHandler::new(
        Arc::clone(&storage) as Arc<dyn PresenceRepository>,
        Arc::clone(&storage) as Arc<dyn NotificationRepository>,
        Arc::clone(&event_bus) as Arc<dyn EventBus>,
        slack_messenger,
    ));
    let dispatcher = InMemoryCommandDispatcher::new(handler);

    // 6. Start the Slack adapter's poll loop now that the EventBus it
    //    publishes to exists. No-ops internally if no credential was found.
    if let Some(adapter) = &slack_adapter {
        adapter
            .start(Arc::clone(&event_bus) as Arc<dyn EventBus>)
            .await?;
        info!("Slack adapter started.");
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
    let ui_registry = Arc::new(InMemoryUiRegistry::new());
    let renderer = TuiRenderer::new(ui_registry, read_model);
    renderer.run_loop().await?;

    info!("Terminal Workspace exited cleanly.");
    Ok(())
}
