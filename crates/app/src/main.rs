//! Main entry point for the Terminal-first Developer Workspace.

use async_trait::async_trait;
use commands::{
    Command, CommandDispatcher, InMemoryCommandDispatcher, Projector, SelectionApplier,
    SharedReadModel, SlackSelectionApplier, WorkspaceCommandHandler,
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
    CalendarAdapter, CalendarConfig, CalendarManager, ConnectionStatus, GitHubAdapter,
    GitHubConfig, IntegrationAdapter, IntegrationConnector, Picker, SlackAdapter, SlackConfig,
    SlackMessenger, SlackPicker,
};
use ipc::{IpcClient, IpcRequest, IpcResponse, IpcServer, IpcStatusProvider, IpcStatusSnapshot};
use logging::{init_logger, spans::application_span};
use notifications::DesktopNotifier;
use plugin_host::{PermissionManager, PluginHostConfig, PluginHostManager, PluginPresenceProvider};
use registry::InMemoryUiRegistry;
use scheduler::AgendaScheduler;
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

/// Calendar's equivalent of [`ConfigFileGitHubSelectionApplier`]
/// (`step24.md`), but with no `config.toml` bridging at all -- unlike
/// GitHub's watched-repository list, every field of a calendar connection
/// (label and URL both) is a secret, so there's nothing non-secret to
/// write to the config file. `CalendarAdapter::keep_only` already handles
/// persistence (via `SecretWriter`) and restarting polling on its own; this
/// wrapper exists purely because `SelectionApplier` is defined in
/// `crates/commands`, which `crates/integration` can't depend on without a
/// cycle (`crates/commands` already depends on `crates/integration`) --
/// same reasoning `ConfigFileGitHubSelectionApplier`'s doc comment gives.
struct CalendarSelectionApplierBridge {
    calendar_adapter: Arc<CalendarAdapter>,
}

#[async_trait]
impl SelectionApplier for CalendarSelectionApplierBridge {
    async fn apply(&self, event_bus: Arc<dyn EventBus>, items: Vec<String>) -> Result<()> {
        self.calendar_adapter.keep_only(event_bus, items).await
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

/// Plain-English rendering of `ConnectionStatus` for `termws status` output
/// (`step15.md` Decision 6) -- deliberately not reusing the TUI's Korean
/// header strings (`crates/ui/src/render.rs`), since this is a separate,
/// English-language CLI surface.
fn connection_status_text(status: ConnectionStatus) -> String {
    match status {
        ConnectionStatus::Disconnected => "Disconnected".to_string(),
        ConnectionStatus::Connecting => "Connecting".to_string(),
        ConnectionStatus::Connected => "Connected".to_string(),
        ConnectionStatus::Reconnecting => "Reconnecting".to_string(),
        ConnectionStatus::Failed(reason) => format!("Failed: {reason}"),
    }
}

/// Supplies `IpcRequest::Status` snapshots (`step15.md` Decision 4) by
/// querying the same adapter handles and `SharedReadModel` `main` already
/// holds -- a fresh `health_check()` per query rather than a cached status,
/// since `termws status` is a one-shot, infrequent call where staleness
/// would be a worse tradeoff than the (cheap, local) extra check.
struct AppStatusProvider {
    read_model: SharedReadModel,
    slack_adapter: Arc<SlackAdapter>,
    github_adapter: Arc<GitHubAdapter>,
    calendar_adapter: Arc<CalendarAdapter>,
}

#[async_trait]
impl IpcStatusProvider for AppStatusProvider {
    async fn snapshot(&self) -> IpcStatusSnapshot {
        let slack = self
            .slack_adapter
            .health_check()
            .await
            .map_or_else(|_| "Unknown".to_string(), connection_status_text);
        let github = self
            .github_adapter
            .health_check()
            .await
            .map_or_else(|_| "Unknown".to_string(), connection_status_text);
        let calendar = self
            .calendar_adapter
            .health_check()
            .await
            .map_or_else(|_| "Unknown".to_string(), connection_status_text);
        let unread_notifications = self.read_model.read().await.unread_notifications.len();
        IpcStatusSnapshot {
            slack,
            github,
            calendar,
            unread_notifications,
        }
    }
}

/// Supplies `get-member-presence` snapshots (`step16.md`) from the same
/// `SharedReadModel` [`AppStatusProvider`] already reads -- no separate
/// data source needed, `DashboardReadModel.team_presence` already has
/// exactly this.
struct AppPresenceProvider {
    read_model: SharedReadModel,
}

#[async_trait]
impl PluginPresenceProvider for AppPresenceProvider {
    async fn presence(&self, user_id: &str) -> Option<PresenceStatus> {
        self.read_model
            .read()
            .await
            .team_presence
            .iter()
            .find(|member| member.user_id.0 == user_id)
            .map(|member| member.status)
    }
}

/// Result of trying to parse `argv` as a `termws <subcommand> ...`
/// invocation (`step15.md` Decision 6).
#[derive(Debug)]
enum CliInvocation {
    /// The first arg isn't one of the three recognized subcommand words at
    /// all -- `main` falls through to its normal TUI bootstrap (e.g. no
    /// args, or `--theme nord`, both real, valid non-subcommand
    /// invocations already handled elsewhere).
    NotASubcommand,
    /// A recognized subcommand word, successfully parsed into a request.
    Request(IpcRequest),
    /// A recognized subcommand word, but with missing/invalid arguments
    /// (e.g. `set-presence bogus`). Distinct from `NotASubcommand`
    /// deliberately -- conflating the two was a real bug found while
    /// testing this manually: `set-presence bogus` fell all the way
    /// through to the full TUI bootstrap (config/storage/adapters) instead
    /// of a clear usage error, and if another instance was already
    /// running, surfaced as a confusing "Database already open" error
    /// instead of "unknown presence status: bogus".
    UsageError(String),
}

/// Parse `termws <subcommand> ...` (`step15.md` Decision 6). `args`
/// excludes the binary name (`argv[0]`).
fn parse_cli_subcommand(args: &[String]) -> CliInvocation {
    match args.first().map(String::as_str) {
        Some("slack-send") => {
            let Some(channel_id) = args.get(1) else {
                return CliInvocation::UsageError(
                    "usage: termws slack-send <channel> <text>".to_string(),
                );
            };
            let Some(text) = args.get(2..).filter(|rest| !rest.is_empty()) else {
                return CliInvocation::UsageError(
                    "usage: termws slack-send <channel> <text>".to_string(),
                );
            };
            CliInvocation::Request(IpcRequest::Dispatch(Command::SendSlackMessage {
                channel_id: channel_id.clone(),
                text: text.join(" "),
            }))
        }
        Some("set-presence") => {
            let Some(raw_status) = args.get(1) else {
                return CliInvocation::UsageError(
                    "usage: termws set-presence <active|away|offline|meeting|lunch> [text]"
                        .to_string(),
                );
            };
            let Some(status) = parse_presence_status(raw_status) else {
                return CliInvocation::UsageError(format!(
                    "unknown presence status: {raw_status} \
                     (expected one of: active, away, offline, meeting, lunch)"
                ));
            };
            let custom_text = args
                .get(2..)
                .filter(|rest| !rest.is_empty())
                .map(|rest| rest.join(" "));
            CliInvocation::Request(IpcRequest::Dispatch(Command::SetPresence {
                status,
                custom_text,
            }))
        }
        Some("status") => CliInvocation::Request(IpcRequest::Status),
        _ => CliInvocation::NotASubcommand,
    }
}

fn parse_presence_status(s: &str) -> Option<PresenceStatus> {
    match s {
        "active" => Some(PresenceStatus::Active),
        "away" => Some(PresenceStatus::Away),
        "offline" => Some(PresenceStatus::Offline),
        "meeting" => Some(PresenceStatus::Meeting),
        "lunch" => Some(PresenceStatus::Lunch),
        _ => None,
    }
}

/// The directory an `IpcServer`/`IpcClient` resolves the socket/pipe
/// under (`step15.md` Decision 5) -- the same directory `config.toml`
/// already lives in, rather than a second directory-resolution scheme.
fn ipc_socket_dir() -> PathBuf {
    config::resolve_config_path()
        .parent()
        .map_or_else(|| PathBuf::from("."), std::path::Path::to_path_buf)
}

/// Run as a one-shot IPC client: connect to an already-running
/// `terminal-workspace` instance, send `request`, print the response, and
/// return the process exit code (`step15.md` Decision 1 -- there is no
/// separate headless daemon to start; if nothing is listening, this fails
/// clearly rather than hanging or silently doing nothing).
async fn run_cli_client(request: IpcRequest) -> i32 {
    let dir = ipc_socket_dir();
    match IpcClient::send(&dir, ipc::DEFAULT_SOCKET_NAME, &request).await {
        Ok(IpcResponse::Ok) => {
            println!("OK");
            0
        }
        Ok(IpcResponse::Status(snapshot)) => {
            println!("Slack: {}", snapshot.slack);
            println!("GitHub: {}", snapshot.github);
            println!("Calendar: {}", snapshot.calendar);
            println!("Unread notifications: {}", snapshot.unread_notifications);
            0
        }
        Ok(IpcResponse::Error(message)) => {
            eprintln!("Error: {message}");
            1
        }
        Err(e) => {
            eprintln!(
                "Could not reach a running terminal-workspace instance: {e}\n\
                 Is it running? (`cargo run -p app` / the installed binary, in another terminal)"
            );
            1
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 0. `termws <subcommand> ...` (`step15.md`, `product-requirements.md`
    //    §4 "Daemon mode & Local CLI Socket IPC") -- checked before any of
    //    the normal bootstrap below (config load, storage, adapters) runs,
    //    since a one-shot CLI-client invocation shouldn't pay any of that
    //    cost: it just needs to talk to an already-running instance and
    //    exit. `args()` excludes argv[0] (the binary path) to match every
    //    other arg-parsing entry point in this file (`resolve_config_path`,
    //    `merge_cli`).
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_cli_subcommand(&args) {
        CliInvocation::Request(request) => std::process::exit(run_cli_client(request).await),
        CliInvocation::UsageError(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
        CliInvocation::NotASubcommand => {}
    }

    // 1. Load layered configuration (Default -> config.toml -> Env -> CLI).
    //    Zero Configuration: creates the config directory/file on first run
    //    if none exists yet (see docs/05-operations/configuration.md §4).
    let config = AppConfig::load_or_create_default()?;

    // 2. Initialize logging using the resolved log level, then enter the
    //    root Application span (see docs/05-operations/logging.md §0).
    //    `init_logger` also returns a `LogBuffer` (`step17.md`) feeding the
    //    TUI's live log panel -- threaded into `TuiRenderer::new` below.
    let log_buffer = init_logger(&config.core.log_level)?;
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

    // 4c. Calendar's equivalent of step 4 (`step12.md`, extended for
    //     multi-calendar support in `step24.md`) — always constructed for
    //     the same "the setup overlay needs something to connect through"
    //     reason. No messenger (read-only, nothing to send). It does have a
    //     picker/selection applier since `step24.md`, but unlike GitHub's
    //     that's a *local* read of already-connected calendars for
    //     removal, not a remote "list my calendars" discovery call -- the
    //     secret-URL auth model still has no such API.
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
    selection_appliers.insert(
        IntegrationSource::Calendar,
        Arc::new(CalendarSelectionApplierBridge {
            calendar_adapter: Arc::clone(&calendar_adapter),
        }),
    );

    let event_bus = Arc::new(InProcessEventBus::new(256));

    // 4d. The Pomodoro timer (`step18.md`) — always constructed (like the
    //     adapters above), starts idle/never-started until a real
    //     `/pomodoro start` command. `run_loop` is spawned once the
    //     dispatcher exists (step 6d below), same lifecycle shape as the
    //     integration adapters' poll loops.
    let scheduler = AgendaScheduler::new(Arc::clone(&event_bus) as Arc<dyn EventBus>);

    let handler = Arc::new(WorkspaceCommandHandler::new(
        Arc::clone(&storage) as Arc<dyn PresenceRepository>,
        Arc::clone(&storage) as Arc<dyn NotificationRepository>,
        Arc::clone(&event_bus) as Arc<dyn EventBus>,
        Some(slack_messenger),
        Some(slack_selection_applier),
        connectors,
        selection_appliers,
        Arc::clone(&scheduler),
        Some(Arc::clone(&calendar_adapter) as Arc<dyn CalendarManager>),
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

    // 6d. The Pomodoro timer's background loop (`step18.md`) -- unlike the
    //     integration adapters above, always started (not gated behind an
    //     `enabled` flag): it stays idle and does nothing until a
    //     `/pomodoro start` command, at which point it needs to already be
    //     running to catch the deadline.
    let scheduler_for_loop = Arc::clone(&scheduler);
    tokio::spawn(async move {
        scheduler_for_loop.run_loop().await;
    });

    // 7. Wire the CQRS read path (Phase 5): Projector keeps a
    //    DashboardReadModel current for the TUI to render from — closes
    //    the read path docs/06-development/decisions/0007-cqrs.md deferred
    //    until a real UI consumer existed.
    let presence_repo = Arc::clone(&storage) as Arc<dyn PresenceRepository>;
    let notification_repo = Arc::clone(&storage) as Arc<dyn NotificationRepository>;
    let (projector, read_model) = Projector::new(&presence_repo, &notification_repo).await?;

    // 7b. Bind the local CLI socket/pipe (`step15.md`, ADR from
    //     `product-requirements.md` §4) -- the running TUI process *is*
    //     the daemon (Decision 1), reachable from a one-shot `termws
    //     slack-send`/`set-presence`/`status` invocation (step 0 above, in
    //     a *different* process). A bind failure (most commonly: another
    //     instance is already running and holds the name) is logged and
    //     otherwise ignored -- IPC is a convenience on top of the TUI, not
    //     a requirement for it to work.
    let ipc_dir = ipc_socket_dir();
    match IpcServer::bind(&ipc_dir, ipc::DEFAULT_SOCKET_NAME) {
        Ok(server) => {
            let status_provider: Arc<dyn IpcStatusProvider> = Arc::new(AppStatusProvider {
                read_model: Arc::clone(&read_model),
                slack_adapter: Arc::clone(&slack_adapter),
                github_adapter: Arc::clone(&github_adapter),
                calendar_adapter: Arc::clone(&calendar_adapter),
            });
            let dispatcher_for_ipc = Arc::clone(&dispatcher);
            tokio::spawn(async move {
                server.serve(dispatcher_for_ipc, status_provider).await;
            });
            info!("IPC socket bound; `termws slack-send`/`set-presence`/`status` can reach this instance.");
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Failed to bind IPC socket (another instance may already be running) -- \
                 the TUI itself is unaffected, but termws CLI subcommands will not reach it."
            );
        }
    }

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
    let plugin_presence_provider: Arc<dyn PluginPresenceProvider> = Arc::new(AppPresenceProvider {
        read_model: Arc::clone(&read_model),
    });
    let plugin_host = Arc::new(PluginHostManager::new(
        PluginHostConfig {
            directory: config.plugins.directory.clone(),
            allowed_list: config.plugins.allowed_list.clone(),
        },
        Arc::clone(&event_bus) as Arc<dyn EventBus>,
        PermissionManager::new(),
        plugin_presence_provider,
    )?);
    if config.plugins.enabled {
        plugin_host.initialize()?;
        plugin_host.load_all().await?;
        event_dispatcher
            .register_handler(Arc::clone(&plugin_host) as Arc<dyn EventHandler>)
            .await;
        info!("Plugin host started.");
    }

    // 8c. Desktop notifications (`step21.md`) -- the whole reason this
    //     exists: know about a Slack DM/GitHub PR/Calendar reminder/
    //     Pomodoro session-end even while working in a different terminal
    //     or app entirely, not only when actually looking at this TUI.
    //     Registered as an `EventHandler` the same way the Projector and
    //     plugin host are above; always-on, no config toggle this phase.
    event_dispatcher
        .register_handler(Arc::new(DesktopNotifier::new()) as Arc<dyn EventHandler>)
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
    //     CQRS read side with no write path of its own. `event_bus` +
    //     the initial status (Phase 9) let the header stay live via a
    //     direct EventBus subscription instead of polling on redraw.
    let initial_slack_status = to_event_status(slack_adapter.health_check().await?);
    let initial_github_status = to_event_status(github_adapter.health_check().await?);
    let initial_calendar_status = to_event_status(calendar_adapter.health_check().await?);
    let ui_registry = Arc::new(InMemoryUiRegistry::new());
    let mut pickers: HashMap<IntegrationSource, Arc<dyn Picker>> = HashMap::new();
    pickers.insert(IntegrationSource::GitHub, github_picker);
    pickers.insert(
        IntegrationSource::Calendar,
        Arc::clone(&calendar_adapter) as Arc<dyn Picker>,
    );
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
        log_buffer,
        scheduler,
        Some(Arc::clone(&calendar_adapter) as Arc<dyn CalendarManager>),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn no_args_is_not_a_subcommand() {
        assert!(matches!(
            parse_cli_subcommand(&args(&[])),
            CliInvocation::NotASubcommand
        ));
    }

    #[test]
    fn an_existing_cli_flag_is_not_mistaken_for_a_subcommand() {
        // `--theme nord` is a real, existing merge_cli flag (config crate)
        // -- must keep falling through to the normal TUI bootstrap, not be
        // misread as an unrecognized subcommand.
        assert!(matches!(
            parse_cli_subcommand(&args(&["--theme", "nord"])),
            CliInvocation::NotASubcommand
        ));
    }

    #[test]
    fn status_parses_to_a_status_request() {
        assert!(matches!(
            parse_cli_subcommand(&args(&["status"])),
            CliInvocation::Request(IpcRequest::Status)
        ));
    }

    #[test]
    fn slack_send_with_channel_and_multi_word_text_parses_correctly() {
        match parse_cli_subcommand(&args(&["slack-send", "#general", "hello", "there"])) {
            CliInvocation::Request(IpcRequest::Dispatch(Command::SendSlackMessage {
                channel_id,
                text,
            })) => {
                assert_eq!(channel_id, "#general");
                assert_eq!(text, "hello there");
            }
            other => panic!("expected a SendSlackMessage dispatch, got: {other:?}"),
        }
    }

    #[test]
    fn slack_send_missing_text_is_a_usage_error_not_a_fallthrough() {
        // The real bug found manually testing this (step15.md Implementation
        // Notes): this used to fall through to `NotASubcommand`, which meant
        // the full TUI bootstrap ran and, if another instance already held
        // the storage lock, failed with a confusing unrelated error instead
        // of a clear usage message.
        assert!(matches!(
            parse_cli_subcommand(&args(&["slack-send", "#general"])),
            CliInvocation::UsageError(_)
        ));
    }

    #[test]
    fn slack_send_missing_channel_is_a_usage_error() {
        assert!(matches!(
            parse_cli_subcommand(&args(&["slack-send"])),
            CliInvocation::UsageError(_)
        ));
    }

    #[test]
    fn set_presence_with_a_valid_status_and_no_text_parses_correctly() {
        match parse_cli_subcommand(&args(&["set-presence", "away"])) {
            CliInvocation::Request(IpcRequest::Dispatch(Command::SetPresence {
                status,
                custom_text,
            })) => {
                assert_eq!(status, PresenceStatus::Away);
                assert_eq!(custom_text, None);
            }
            other => panic!("expected a SetPresence dispatch, got: {other:?}"),
        }
    }

    #[test]
    fn set_presence_with_a_valid_status_and_custom_text_parses_correctly() {
        match parse_cli_subcommand(&args(&["set-presence", "lunch", "back", "at", "1"])) {
            CliInvocation::Request(IpcRequest::Dispatch(Command::SetPresence {
                status,
                custom_text,
            })) => {
                assert_eq!(status, PresenceStatus::Lunch);
                assert_eq!(custom_text, Some("back at 1".to_string()));
            }
            other => panic!("expected a SetPresence dispatch, got: {other:?}"),
        }
    }

    #[test]
    fn set_presence_with_an_unknown_status_word_is_a_usage_error_not_a_fallthrough() {
        assert!(matches!(
            parse_cli_subcommand(&args(&["set-presence", "bogus"])),
            CliInvocation::UsageError(_)
        ));
    }

    #[test]
    fn set_presence_missing_status_is_a_usage_error() {
        assert!(matches!(
            parse_cli_subcommand(&args(&["set-presence"])),
            CliInvocation::UsageError(_)
        ));
    }
}
