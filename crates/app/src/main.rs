//! Main entry point for the Terminal-first Developer Workspace.

use commands::{
    Command, CommandDispatcher, InMemoryCommandDispatcher, Projector, WorkspaceCommandHandler,
};
use common::Result;
use config::AppConfig;
use domain::{FailedEventRepository, NotificationRepository, PresenceRepository, PresenceStatus};
use events::{EventBus, EventDispatcher, EventHandler, InProcessEventBus};
use integration::{
    ConnectionStatus, IntegrationAdapter, SlackAdapter, SlackConfig, SlackConnector, SlackMessenger,
};
use logging::{init_logger, spans::application_span};
use registry::InMemoryUiRegistry;
use secrets::{SecretProviderChain, SecretWriter};
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
    let slack_connector = Arc::clone(&slack_adapter) as Arc<dyn SlackConnector>;

    // 5. Wire the CQRS write path: Command -> WorkspaceCommandHandler ->
    //    Storage + EventBus (see docs/06-development/decisions/0007-cqrs.md).
    let event_bus = Arc::new(InProcessEventBus::new(256));
    let handler = Arc::new(WorkspaceCommandHandler::new(
        Arc::clone(&storage) as Arc<dyn PresenceRepository>,
        Arc::clone(&storage) as Arc<dyn NotificationRepository>,
        Arc::clone(&event_bus) as Arc<dyn EventBus>,
        Some(slack_messenger),
        Some(slack_connector),
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
    //     Passing `dispatcher` (Phase 7) is what lets the setup overlay's
    //     `Ctrl+S` actually mutate anything — before this the TUI was pure
    //     CQRS read side with no write path of its own.
    let ui_registry = Arc::new(InMemoryUiRegistry::new());
    let renderer = TuiRenderer::new(ui_registry, read_model, Arc::clone(&dispatcher));
    renderer.run_loop().await?;

    info!("Terminal Workspace exited cleanly.");
    Ok(())
}
