//! Main entry point for the Terminal-first Developer Workspace.

use commands::{Command, CommandDispatcher, InMemoryCommandDispatcher, WorkspaceCommandHandler};
use common::Result;
use config::AppConfig;
use domain::{FailedEventRepository, NotificationRepository, PresenceRepository, PresenceStatus};
use events::{EventBus, EventDispatcher, InProcessEventBus};
use logging::{init_logger, spans::application_span};
use std::sync::Arc;
use storage::RedbStorageBackend;
use tracing::info;

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

    // 4. Wire the CQRS write path: Command -> WorkspaceCommandHandler ->
    //    Storage + EventBus (see docs/06-development/decisions/0007-cqrs.md).
    let event_bus = Arc::new(InProcessEventBus::new(256));
    let handler = Arc::new(WorkspaceCommandHandler::new(
        Arc::clone(&storage) as Arc<dyn PresenceRepository>,
        Arc::clone(&storage) as Arc<dyn NotificationRepository>,
        Arc::clone(&event_bus) as Arc<dyn EventBus>,
    ));
    let dispatcher = InMemoryCommandDispatcher::new(handler);

    // 5. Wire event reliability: retry/backoff + Dead Letter Queue (see
    //    docs/06-development/decisions/0003-event-bus.md's Phase 3 amendment).
    let event_dispatcher = EventDispatcher::new(Arc::clone(&event_bus) as Arc<dyn EventBus>)
        .with_dlq(Arc::clone(&storage) as Arc<dyn FailedEventRepository>);
    event_dispatcher.start();

    // 6. Prove the write path end-to-end with a startup presence command.
    dispatcher
        .dispatch(Command::SetPresence {
            status: PresenceStatus::Active,
            custom_text: None,
        })
        .await?;
    info!("CQRS write path verified: startup SetPresence command dispatched.");

    // 7. Stub main exit
    info!("Skeleton execution finished successfully.");
    Ok(())
}
