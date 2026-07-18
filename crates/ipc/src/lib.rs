//! Local CLI socket IPC (`step15.md`, `product-requirements.md` §4). The
//! running interactive `terminal-workspace` process *is* the daemon
//! (Decision 1) -- this crate provides the socket/pipe transport
//! (Decision 3, via `interprocess`) and framing (Decision 4) that let a
//! one-shot `termws <subcommand>` CLI invocation reach it.

use async_trait::async_trait;
use commands::{Command, CommandDispatcher};
use common::WorkspaceError;
use interprocess::local_socket::{
    tokio::{prelude::*, Listener, Stream},
    GenericFilePath, GenericNamespaced, ListenerOptions, ToFsName, ToNsName,
};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// The name every real `terminal-workspace` instance binds/connects to
/// (Decision 5) -- fixed and predictable so a one-shot `termws <subcommand>`
/// CLI invocation can find the running instance without a discovery step or
/// any config. A second instance's [`IpcServer::bind`] failing with
/// `AddrInUse` on this name is expected and acceptable: IPC is a
/// convenience on top of the single already-running workspace (Decision 1),
/// not a multi-instance broker.
pub const DEFAULT_SOCKET_NAME: &str = "terminal-workspace-ipc";

/// Resolve `name` to a socket/pipe address -- an abstract namespaced name
/// where the platform supports one (Windows named pipes, Linux abstract
/// Unix sockets), falling back to a real filesystem path under `dir`
/// otherwise (macOS, and any Unix without abstract-namespace support).
/// `dir` is the caller's resolved config/runtime directory (`crates/app`
/// passes the same directory `config::resolve_config_path` already
/// resolves to, rather than this crate inventing a second
/// directory-resolution scheme); `name` is [`DEFAULT_SOCKET_NAME`] in
/// production, or a per-test-unique name in this crate's own tests (real
/// sockets are process/system-global, so parallel `cargo test` runs would
/// otherwise collide on a fixed name).
fn socket_name(
    dir: &std::path::Path,
    name: &str,
) -> io::Result<interprocess::local_socket::Name<'static>> {
    if GenericNamespaced::is_supported() {
        // Owned `String` (not `&str`) so the resulting `Name` doesn't
        // borrow from `name`, which may be shorter-lived than the
        // `Listener`/connection this name is used to create.
        name.to_string().to_ns_name::<GenericNamespaced>()
    } else {
        let path: PathBuf = dir.join(format!("{name}.sock"));
        path.to_fs_name::<GenericFilePath>()
    }
}

/// One request sent from an `IpcClient` to the `IpcServer` (Decision 4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcRequest {
    /// Fire-and-forget write, dispatched through the same
    /// `CommandDispatcher` every TUI keystroke already goes through.
    Dispatch(Command),
    /// Read-only query: connection statuses + unread notification count.
    Status,
}

/// One response sent from the `IpcServer` back to an `IpcClient`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    /// A `Dispatch` request's command completed successfully.
    Ok,
    /// Snapshot answering a `Status` request.
    Status(IpcStatusSnapshot),
    /// Something failed -- a dispatch error, a malformed request, etc.
    Error(String),
}

/// A point-in-time snapshot for `termws status` (Decision 6). Kept
/// string-typed rather than reusing `events::IntegrationConnectionStatus`
/// deliberately -- this crate has no reason to depend on
/// `crates/events`/`crates/integration` just to render three status words
/// and a count; the caller (`crates/app`, via [`IpcStatusProvider`])
/// already has richer status types and reduces them to display strings
/// here, the same way the TUI's header line already does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcStatusSnapshot {
    /// Slack connection status, as displayed text (e.g. "Connected").
    pub slack: String,
    /// GitHub connection status, as displayed text.
    pub github: String,
    /// Calendar connection status, as displayed text.
    pub calendar: String,
    /// Count of unread notifications across every source.
    pub unread_notifications: usize,
}

/// Supplies the live data behind `IpcRequest::Status`. Implemented in
/// `crates/app` (the only place that actually holds adapter health-check
/// handles and the `SharedReadModel`) and injected into [`IpcServer`], so
/// this crate stays decoupled from adapter-level types.
#[async_trait]
pub trait IpcStatusProvider: Send + Sync {
    /// Produce a fresh snapshot for a `Status` request.
    async fn snapshot(&self) -> IpcStatusSnapshot;
}

/// Binds the socket/pipe and serves `IpcRequest`s for the lifetime of the
/// running process (Decision 1 -- the interactive TUI process is the
/// daemon).
pub struct IpcServer {
    listener: Listener,
}

impl IpcServer {
    /// Bind the socket/pipe named `name` under `dir` (see [`socket_name`]).
    /// Does not accept connections yet -- see [`Self::serve`]. Production
    /// callers should pass [`DEFAULT_SOCKET_NAME`]; this crate's own tests
    /// pass a per-test-unique name to avoid colliding on the real,
    /// process/system-global socket namespace.
    pub fn bind(dir: &std::path::Path, name: &str) -> io::Result<Self> {
        let name = socket_name(dir, name)?;
        let listener = ListenerOptions::new().name(name).create_tokio()?;
        Ok(Self { listener })
    }

    /// Accept connections forever, handling each on its own task. Never
    /// returns under normal operation; a per-connection error is logged
    /// and that connection alone is dropped, matching this project's
    /// "one bad actor must not take down the rest of the system" pattern
    /// (ADR-0002's plugin-trap handling, `EventDispatcher`'s per-handler
    /// isolation).
    pub async fn serve(
        &self,
        dispatcher: Arc<dyn CommandDispatcher>,
        status: Arc<dyn IpcStatusProvider>,
    ) {
        loop {
            let conn = match self.listener.accept().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "IPC listener accept failed");
                    continue;
                }
            };
            let dispatcher = Arc::clone(&dispatcher);
            let status = Arc::clone(&status);
            tokio::spawn(async move {
                if let Err(e) = handle_connection(conn, dispatcher, status).await {
                    tracing::warn!(error = %e, "IPC connection handling failed");
                }
            });
        }
    }
}

async fn handle_connection(
    conn: Stream,
    dispatcher: Arc<dyn CommandDispatcher>,
    status: Arc<dyn IpcStatusProvider>,
) -> io::Result<()> {
    let mut reader = BufReader::new(&conn);
    let mut writer = &conn;

    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let response = match serde_json::from_str::<IpcRequest>(line.trim_end()) {
        Ok(IpcRequest::Dispatch(command)) => match dispatcher.dispatch(command).await {
            Ok(()) => IpcResponse::Ok,
            Err(e) => IpcResponse::Error(e.to_string()),
        },
        Ok(IpcRequest::Status) => IpcResponse::Status(status.snapshot().await),
        Err(e) => IpcResponse::Error(format!("malformed request: {e}")),
    };

    let mut body = serde_json::to_string(&response).unwrap_or_else(|e| {
        serde_json::to_string(&IpcResponse::Error(format!(
            "response serialization failed: {e}"
        )))
        .expect("a hand-written IpcResponse::Error always serializes")
    });
    body.push('\n');
    writer.write_all(body.as_bytes()).await
}

/// Connects to an already-running [`IpcServer`], sends exactly one
/// [`IpcRequest`], and reads exactly one [`IpcResponse`] back.
pub struct IpcClient;

impl IpcClient {
    /// Send `request` and return the server's response. `WorkspaceError::
    /// Integration` on any connection/protocol failure, wrapping the
    /// underlying `io::Error` -- most commonly "no instance running"
    /// (connection refused / name not found), which the CLI-client caller
    /// in `crates/app` turns into a clear message rather than a raw OS
    /// error string.
    pub async fn send(
        dir: &std::path::Path,
        name: &str,
        request: &IpcRequest,
    ) -> common::Result<IpcResponse> {
        let name =
            socket_name(dir, name).map_err(|e| WorkspaceError::Integration(e.to_string()))?;
        let conn = Stream::connect(name)
            .await
            .map_err(|e| WorkspaceError::Integration(e.to_string()))?;

        let mut writer = &conn;
        let mut body = serde_json::to_string(request)
            .map_err(|e| WorkspaceError::Integration(e.to_string()))?;
        body.push('\n');
        writer
            .write_all(body.as_bytes())
            .await
            .map_err(|e| WorkspaceError::Integration(e.to_string()))?;

        let mut reader = BufReader::new(&conn);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| WorkspaceError::Integration(e.to_string()))?;

        serde_json::from_str(line.trim_end())
            .map_err(|e| WorkspaceError::Integration(format!("malformed response: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::Result;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct MockDispatcher {
        received: Mutex<Vec<Command>>,
        fail_with: Option<String>,
    }

    #[async_trait]
    impl CommandDispatcher for MockDispatcher {
        async fn dispatch(&self, command: Command) -> Result<()> {
            self.received.lock().await.push(command);
            match &self.fail_with {
                Some(msg) => Err(WorkspaceError::Integration(msg.clone())),
                None => Ok(()),
            }
        }
    }

    struct MockStatusProvider(IpcStatusSnapshot);

    #[async_trait]
    impl IpcStatusProvider for MockStatusProvider {
        async fn snapshot(&self) -> IpcStatusSnapshot {
            self.0.clone()
        }
    }

    fn unique_name(label: &str) -> String {
        format!("tw-ipc-test-{label}-{}", uuid::Uuid::new_v4())
    }

    #[tokio::test]
    async fn a_real_client_dispatches_through_a_real_server_over_a_real_socket() {
        let name = unique_name("dispatch");
        let dir = std::env::temp_dir();
        let server = IpcServer::bind(&dir, &name).expect("bind must succeed");
        let dispatcher: Arc<MockDispatcher> = Arc::default();
        let status = Arc::new(MockStatusProvider(IpcStatusSnapshot {
            slack: "Connected".into(),
            github: "Disconnected".into(),
            calendar: "Disconnected".into(),
            unread_notifications: 0,
        }));

        let dispatcher_for_server = Arc::clone(&dispatcher) as Arc<dyn CommandDispatcher>;
        let status_for_server = Arc::clone(&status) as Arc<dyn IpcStatusProvider>;
        tokio::spawn(async move {
            server.serve(dispatcher_for_server, status_for_server).await;
        });
        // Give the server task a moment to reach `accept().await`.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let request = IpcRequest::Dispatch(Command::SendSlackMessage {
            channel_id: "#general".into(),
            text: "hello from a real socket".into(),
        });
        let response = IpcClient::send(&dir, &name, &request)
            .await
            .expect("send must succeed against a real running server");

        assert!(matches!(response, IpcResponse::Ok));
        let received = dispatcher.received.lock().await;
        assert_eq!(received.len(), 1);
        assert!(
            matches!(&received[0], Command::SendSlackMessage { channel_id, text }
            if channel_id == "#general" && text == "hello from a real socket")
        );
    }

    #[tokio::test]
    async fn a_real_client_receives_a_real_status_snapshot() {
        let name = unique_name("status");
        let dir = std::env::temp_dir();
        let server = IpcServer::bind(&dir, &name).expect("bind must succeed");
        let dispatcher: Arc<MockDispatcher> = Arc::default();
        let expected = IpcStatusSnapshot {
            slack: "Connected".into(),
            github: "Reconnecting".into(),
            calendar: "Disconnected".into(),
            unread_notifications: 7,
        };
        let status = Arc::new(MockStatusProvider(expected.clone()));

        let dispatcher_for_server = Arc::clone(&dispatcher) as Arc<dyn CommandDispatcher>;
        let status_for_server = Arc::clone(&status) as Arc<dyn IpcStatusProvider>;
        tokio::spawn(async move {
            server.serve(dispatcher_for_server, status_for_server).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let response = IpcClient::send(&dir, &name, &IpcRequest::Status)
            .await
            .expect("send must succeed");

        match response {
            IpcResponse::Status(snapshot) => assert_eq!(snapshot, expected),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_dispatch_failure_is_reported_back_to_the_client_not_swallowed() {
        let name = unique_name("failure");
        let dir = std::env::temp_dir();
        let server = IpcServer::bind(&dir, &name).expect("bind must succeed");
        let dispatcher = Arc::new(MockDispatcher {
            received: Mutex::new(Vec::new()),
            fail_with: Some("channel not found".to_string()),
        });
        let status = Arc::new(MockStatusProvider(IpcStatusSnapshot {
            slack: "Connected".into(),
            github: "Connected".into(),
            calendar: "Connected".into(),
            unread_notifications: 0,
        }));

        let dispatcher_for_server = Arc::clone(&dispatcher) as Arc<dyn CommandDispatcher>;
        let status_for_server = Arc::clone(&status) as Arc<dyn IpcStatusProvider>;
        tokio::spawn(async move {
            server.serve(dispatcher_for_server, status_for_server).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let request = IpcRequest::Dispatch(Command::SendSlackMessage {
            channel_id: "#nowhere".into(),
            text: "hi".into(),
        });
        let response = IpcClient::send(&dir, &name, &request)
            .await
            .expect("the socket round-trip itself must still succeed");

        match response {
            IpcResponse::Error(msg) => assert!(msg.contains("channel not found")),
            other => {
                panic!("expected an Error response carrying the dispatch failure, got {other:?}")
            }
        }
    }

    #[tokio::test]
    async fn connecting_to_a_name_nothing_is_listening_on_fails_clearly() {
        let name = unique_name("nobody-home");
        let dir = std::env::temp_dir();

        let result = IpcClient::send(&dir, &name, &IpcRequest::Status).await;

        assert!(result.is_err());
    }

    #[test]
    fn ipc_request_dispatch_round_trips_through_json() {
        let req = IpcRequest::Dispatch(Command::SendSlackMessage {
            channel_id: "C1".into(),
            text: "hi".into(),
        });
        let json = serde_json::to_string(&req).unwrap();
        let back: IpcRequest = serde_json::from_str(&json).unwrap();
        match back {
            IpcRequest::Dispatch(Command::SendSlackMessage { channel_id, text }) => {
                assert_eq!(channel_id, "C1");
                assert_eq!(text, "hi");
            }
            other => panic!("unexpected round-trip result: {other:?}"),
        }
    }

    #[test]
    fn ipc_request_status_round_trips_through_json() {
        let json = serde_json::to_string(&IpcRequest::Status).unwrap();
        assert!(matches!(
            serde_json::from_str::<IpcRequest>(&json).unwrap(),
            IpcRequest::Status
        ));
    }

    #[test]
    fn ipc_response_status_round_trips_through_json() {
        let snapshot = IpcStatusSnapshot {
            slack: "Connected".into(),
            github: "Disconnected".into(),
            calendar: "Reconnecting".into(),
            unread_notifications: 3,
        };
        let resp = IpcResponse::Status(snapshot.clone());
        let json = serde_json::to_string(&resp).unwrap();
        match serde_json::from_str::<IpcResponse>(&json).unwrap() {
            IpcResponse::Status(back) => assert_eq!(back, snapshot),
            other => panic!("unexpected round-trip result: {other:?}"),
        }
    }

    #[test]
    fn ipc_response_error_round_trips_through_json() {
        let resp = IpcResponse::Error("boom".into());
        let json = serde_json::to_string(&resp).unwrap();
        match serde_json::from_str::<IpcResponse>(&json).unwrap() {
            IpcResponse::Error(msg) => assert_eq!(msg, "boom"),
            other => panic!("unexpected round-trip result: {other:?}"),
        }
    }

    #[test]
    fn a_malformed_request_line_deserializes_to_an_error_not_a_panic() {
        let result = serde_json::from_str::<IpcRequest>("not json");
        assert!(result.is_err());
    }
}
