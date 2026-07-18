# Implementation Plan - Phase 15: Daemon Mode & Local CLI Socket IPC (v1.0)

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-14.

## Context

`docs/01-product/product-requirements.md` §4 lists **"Daemon mode & Local CLI Socket IPC"** as a v1.0.0 release-scope item, alongside the WASM plugin runtime (done, Phase 14), multi-integration adapters (done, Phases 6/10/12), and `redb` storage (done, Phase 3). Unlike those, this item has **no existing design or code at all** — `crates/app/src/main.rs` only ever runs the interactive TUI in the foreground; there is no socket, no background mode, no CLI subcommand surface beyond `config::ConfigBuilder::merge_cli`'s tiny `--theme`/`--log-level`/`--refresh-rate-ms`/`--config` flag scanner.

The concrete scenario already exists, though — `docs/01-product/user-flows.md` §3 ("Vim / Tmux Shell IPC"):

> While editing code inside Vim, a developer executes `:!termws slack-send "@bob" "Here is the patch"`. The local IPC client pushes this command to the running background workspace daemon via Unix Domain Sockets (Linux/macOS) or Named Pipes (Windows), avoiding full app swaps.

This fixes the transport choice (already decided, not re-litigated here): Unix Domain Sockets on Linux/macOS, Named Pipes on Windows.

Two things already in the codebase make this a much smaller lift than it sounds:
- `commands::Command` (`crates/commands/src/lib.rs`) already derives `Serialize`/`Deserialize` and is **not** frozen by Architecture Freeze v1 (`development.md` §3) — it's a natural fit for a wire protocol, not something that needs a parallel DTO.
- `commands::CommandDispatcher::dispatch(&self, command: Command) -> Result<()>` is the existing, already-tested write-path entry point every TUI keystroke already funnels through. An IPC handler doing the same thing is not a new architectural seam, just a new caller of one that already exists.

---

## Decisions

### 1. Process model: the running TUI *is* the daemon

**Confirmed**: no separate headless `--daemon` process, no OS service installation (systemd unit / launchd plist / Windows service). The already-interactive `terminal-workspace` binary binds the IPC socket/pipe for its entire run, in the background, whenever it's running — "daemon mode" means *the app you already have open in a terminal pane is reachable via IPC*, not *a second invisible process*. If no instance is running, an IPC client attempt fails with a clear "workspace not running" error rather than silently doing nothing or auto-spawning one.

**Why**: matches how this tool is actually used (open in a pane while you work, per the whole "Terminal Workspace" concept) and "Zero Setup" (`product-requirements.md` §2.1) — no service installation step, nothing to configure. A true auto-starting background service is a materially bigger scope (process supervision, per-OS service-manager integration, start-on-login) that the PRD line item doesn't actually ask for; it says the running app is reachable via IPC, not that the app must always be running.

**Alternative considered**: separate `termws --daemon` headless process the user starts once (e.g. via their shell profile or a system service), with the interactive TUI becoming just one possible client of it. Rejected for this phase as over-scoped relative to what the PRD/user-flow actually describes — worth revisiting as a real feature later if someone wants IPC access without ever opening the TUI.

### 2. New `crates/ipc` crate

**Confirmed**: a new crate, `crates/ipc`, housing the socket/pipe transport (cross-platform abstraction), request/response framing, and (de)serialization — mirroring how `plugin-host`/`plugin-sdk` each got their own crate in Phase 14 rather than living inside `app`. `crates/app/src/main.rs` constructs the IPC server and wires it to the existing `Arc<dyn CommandDispatcher>` and the existing `DashboardReadModel`/connection-status handles, the same way it already wires the TUI.

### 3. Transport: the `interprocess` crate, not a hand-rolled `cfg(windows)` split

**Confirmed**: `interprocess` (`v2.4.2`, pure-Rust cross-platform local-socket abstraction, `tokio` feature for async support) instead of hand-rolling `tokio::net::UnixListener` (Linux/macOS) plus a separate `tokio::net::windows::named_pipe` implementation (Windows). **Verified before proposing it** (this project's standing practice since ADR-0014): `cargo add interprocess -p app --features tokio` then `cargo tree -p app -i cc` shows `cc` only via the already-accepted `wasmtime` path (ADR-0017) — `interprocess` itself adds no new C-compiler requirement. Change reverted after verification; not yet added for real.

**Why**: named-pipe security descriptors and connection semantics on Windows are a well-known place to introduce subtle bugs by hand; a maintained abstraction covering both platforms with one API is less risk than two bespoke code paths for something this fiddly, and it doesn't cost anything ADR-0017 hasn't already paid for.

### 4. Wire protocol: newline-delimited JSON, a small envelope enum — not full JSON-RPC

**Confirmed (not separately asked — low-stakes, entailed by Decision 2)**: each request/response is one JSON object terminated by `\n` (matches this project's existing "everything already speaks JSON" pattern — `Event` serialization, `Command` serialization). A new `IpcRequest`/`IpcResponse` envelope in `crates/ipc`:

```rust
enum IpcRequest {
    Dispatch(commands::Command),   // fire-and-forget write, e.g. slack-send/set-presence
    Status,                        // read-only query: connection statuses, unread count
}

enum IpcResponse {
    Ok,
    Status { slack: String, github: String, calendar: String, unread_notifications: usize },
    Error(String),
}
```

**Why not full JSON-RPC 2.0**: single local client at a time in the common case, no method-discovery/batching/notification-vs-request distinction needed — pulling in a JSON-RPC crate (or hand-rolling the envelope fields it requires) buys nothing this local single-purpose protocol needs. Consistent with `ConfigBuilder::merge_cli`'s existing "intentionally tiny, revisit via ADR if the surface actually grows" reasoning.

### 5. Socket/pipe location & access control

**Confirmed (not separately asked — low-stakes, entailed by Decisions 1/3)**: Unix socket at `$XDG_RUNTIME_DIR/terminal-workspace/ipc.sock` (falling back to the same config-dir resolution `config::resolve_config_path` already uses if `XDG_RUNTIME_DIR` is unset — consistent fallback pattern, not a new one). Windows named pipe at `\\.\pipe\terminal-workspace`. No application-level auth: relies on OS-level access control (Unix socket file permissions created `0600`; Windows named pipes default to same-user access) — matches the existing `LOCAL_USER_ID` placeholder comment in `crates/commands/src/lib.rs` ("no auth/identity system exists yet... revisit once multi-user or authenticated scenarios are in scope").

### 6. CLI subcommand surface for this phase (deliberately narrow)

**Confirmed**: exactly three subcommands, chosen because they're the ones that make sense as a fire-and-forget one-shot CLI call (as opposed to something inherently interactive, like picking from a list or typing a secret token):

- `termws slack-send <channel> <text>` → `IpcRequest::Dispatch(Command::SendSlackMessage { .. })`
- `termws set-presence <status> [text]` → `IpcRequest::Dispatch(Command::SetPresence { .. })`
- `termws status` → `IpcRequest::Status`, prints connection status per integration + unread count

**Deliberately not included**: `Connect`/`ApplySelection`/`ApplySlackSelection` — these involve entering secrets or picking from a fetched list, which the TUI's overlays already do well and a one-shot CLI call would do awkwardly (secrets on the command line/shell history is also a real security downgrade from the existing OS-keyring flow). Add more subcommands once a real use case needs one, not speculatively.

When `terminal-workspace` is invoked with one of these subcommand forms, `main.rs` runs as a thin IPC **client** (connect, send one request, print the response, exit) instead of starting the TUI — mirrors how `merge_cli` already distinguishes flag-style args; this adds subcommand-style dispatch alongside it.

---

## Proposed Changes (pending confirmation of Decisions 1-6 above)

#### [NEW] `crates/ipc` crate
- `IpcRequest`/`IpcResponse` envelope types (Decision 4).
- `IpcServer`: binds the socket/pipe (Decision 5), accepts connections, deserializes one `IpcRequest` per line, dispatches `Dispatch` via an injected `Arc<dyn CommandDispatcher>` or answers `Status` from an injected read-side handle, serializes one `IpcResponse` per line back.
- `IpcClient`: connects, sends one `IpcRequest`, reads one `IpcResponse`, for the CLI-client side of `main.rs`.
- Cross-platform transport via `interprocess` (Decision 3).

#### [MODIFY] `crates/app/Cargo.toml`, `crates/app/src/main.rs`
- Add `ipc = { path = "../ipc" }` dependency.
- Parse `std::env::args()` for the three subcommand forms (Decision 6) *before* the existing config/logging/storage bootstrap — a CLI-client invocation shouldn't pay the cost of opening `redb`, starting adapters, etc., it just needs to talk to an already-running instance and exit.
- If a subcommand matched: build an `IpcClient`, send, print, exit with the dispatcher's success/failure as the process exit code.
- Otherwise (existing path, unchanged): full TUI bootstrap as today, plus construct and start an `IpcServer` bound to the existing `dispatcher`/`read_model`/connection-status handles, alongside the existing `event_dispatcher`/`renderer.run_loop()`.

#### [MODIFY] `docs/01-product/product-requirements.md`
Mark this release-scope line as implemented once done, matching how other v1.0.0 lines will read.

---

## Verification Plan

- Unit tests for `IpcRequest`/`IpcResponse` JSON round-tripping (mirrors every prior phase's serde round-trip test pattern).
- An integration test in `crates/ipc` spinning up a real `IpcServer` bound to a mock `CommandDispatcher`/read model, connecting a real `IpcClient` against it (over an actual OS socket/pipe, not mocked), and asserting a dispatched `slack-send` actually reaches the mock dispatcher and a `status` query returns the expected snapshot.
- Manual verification: run `cargo run -p app` in one terminal, `cargo run -p app -- slack-send '#general' 'hello'` in another, confirm the message actually appears (with a real Slack connection configured) and the exit code is 0; confirm `cargo run -p app -- status` with no instance running fails clearly rather than hanging.
- No Windows-specific CI verification beyond the existing 3-OS matrix already exercising `cargo test --workspace` — `interprocess`'s own test suite covers the low-level named-pipe correctness; this project's tests only need to prove *this crate's* usage of it works.

---

## Implementation Notes (what actually happened)

Every Proposed Change above was implemented and verified — including real manual end-to-end testing against a live running instance, not just the automated suite. `cargo fmt --all --check`, `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` all pass clean on the native `x86_64-pc-windows-gnu` target.

1. **`interprocess`'s `local_socket::tokio` API needed real inspection, not guessing.** The crate's own bundled examples (`examples/local_socket/tokio/{listener,stream}.rs`, found in the local `cargo` registry cache the same way `wasmtime`'s `bindgen_examples` were inspected in Phase 14) gave the exact shape: `GenericNamespaced`/`GenericFilePath` name types, `ListenerOptions::new().name(name).create_tokio()`, `Stream::connect(name)`. Using `GenericNamespaced` (falling back to `GenericFilePath` only where the platform lacks an abstract namespace) turned out to be a cleaner fit than the original Decision 5 sketch's literal `\\.\pipe\terminal-workspace` / `$XDG_RUNTIME_DIR/.../ipc.sock` paths — `interprocess` abstracts that platform difference itself; this crate only supplies a name string.

2. **Socket names had to be parameterized, not hardcoded, once real tests needed one.** The first version hardcoded a single `"terminal-workspace-ipc"` name inside `socket_name()`. Real sockets are process/system-global — `cargo test`'s parallel test execution immediately collided on `AddrInUse` across the crate's own integration tests. Fixed by threading a `name: &str` parameter through `IpcServer::bind`/`IpcClient::send`/`socket_name`, with `ipc::DEFAULT_SOCKET_NAME` for production use and a per-test-unique (`uuid`-suffixed) name in this crate's own tests.

3. **A lifetime mismatch surfaced from `ToNsName`'s borrow-vs-owned distinction.** `socket_name`'s declared return type is `Name<'static>` (so `IpcServer`/`IpcClient` don't need to thread a borrowed name's lifetime through their own signatures), but `&str::to_ns_name()` ties the returned `Name`'s lifetime to the borrow. Fixed by converting to an owned `String` first (`name.to_string().to_ns_name::<GenericNamespaced>()`) — `ToNsName`'s owned-`String` impl isn't tied to any borrow, letting inference pick `'static`.

4. **A real, manually-found bug**: the first version of `parse_cli_subcommand` returned `Option<IpcRequest>`, conflating two different situations under one `None` — "the first arg isn't a recognized subcommand at all" (correctly falls through to the normal TUI bootstrap) and "it *is* a recognized subcommand but has invalid/missing arguments" (should print a clear usage error and exit immediately). Running `termws set-presence bogus` against a real already-running instance exposed this concretely: instead of a clear "unknown presence status: bogus" message, it fell through to the **full TUI bootstrap** (config load, `redb` open, adapter init), which then failed with a confusing, unrelated `Storage("Database already open. Cannot acquire lock.")` — the real error (bad argument) was nowhere in that message. Fixed by replacing the `Option<IpcRequest>` return with a three-way `CliInvocation` enum (`NotASubcommand` / `Request` / `UsageError`), with `main` handling `UsageError` by printing the message and exiting `2` before any bootstrap work happens. Ten unit tests added directly to `crates/app/src/main.rs` (a binary crate can have its own `#[cfg(test)]` module) cover this distinction, including a regression test named specifically for the bug found.

5. **Full manual end-to-end verification against a real running instance**, per this phase's Verification Plan and this project's standing "verify empirically" discipline: started a real `terminal-workspace.exe` instance, confirmed it logs "IPC socket bound," then from a separate process invocation confirmed all three subcommands for real: `termws status` returned a live snapshot (`Slack: Disconnected`, `GitHub: Disconnected`, `Calendar: Connecting`, reflecting this session's actual prior test configuration — not fixture data); `termws set-presence active` dispatched successfully (`OK`, exit 0) and was independently confirmed via the CQRS write path already proven by `cqrs_slice_test.rs`; `termws slack-send '#general' hi there` correctly surfaced a real dispatch failure (`Error: Integration error: Slack is not configured (no token found)`, exit 1) since no Slack token was configured on that instance; `termws status` with no instance running failed clearly (`Could not reach a running terminal-workspace instance: ...`, exit 1) rather than hanging, confirming Decision 1's "fails clearly, not silently" requirement for real.

6. **An unrelated environment issue surfaced during manual testing, not a bug in this phase's code**: a `terminal-workspace.exe` process left running from earlier session testing (unrelated to IPC) held the `redb` storage lock and, separately, the Windows exe file lock (blocking rebuilds). Identified via `Get-Process terminal-workspace` and cleared with `Stop-Process` before verification could proceed — a reminder that this project's manual-testing steps need a clean process state, not a defect in the IPC feature itself.

Deferred, as scoped by the Decisions above: a genuinely separate headless daemon process (Decision 1's rejected alternative) and any subcommand beyond the three in Decision 6 (e.g. GitHub/Calendar-specific one-shot commands) — add if a real use case needs one, not speculatively.
