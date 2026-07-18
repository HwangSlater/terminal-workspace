# Implementation Plan - Phase 21: Desktop (OS) Notifications

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-20.

## Context

User feedback, stated directly as one of the core reasons this project exists: while working in a different terminal (or a different app entirely), there is currently **no way to learn that something happened** in Terminal Workspace — a Slack DM, a GitHub review request, a Calendar reminder, a finished Pomodoro session — without switching back to look at the TUI window, or manually running `termws status` (`step15.md`'s IPC CLI, a pull, not a push). The Pomodoro terminal bell (`step18.md` Decision 3) has the same blind spot: it only fires into the terminal the TUI is actually attached to, so it's silent if that terminal isn't the one currently in front of the user.

This is a real, unmet product goal, not a polish item — it's the difference between "a dashboard you have to remember to check" and "a workspace that tells you when something needs you."

**Technical feasibility verified before proposing anything** (this project's standing practice since ADR-0014/`step15.md`): `cargo add notify-rust -p app` (temporarily, then reverted) followed by `cargo tree -p app -i cc` shows `cc` only via the already-accepted `wasmtime` path (ADR-0017) — `notify-rust` itself adds no new C-compiler requirement. `cargo check -p app` with it added compiled cleanly on this Windows machine. `notify-rust` v4.18.0 is cross-platform: `tauri-winrt-notification` (pulls in the `windows` crate family, no C compiler) on Windows, D-Bus/`zbus` on Linux, and Cocoa's notification APIs on macOS.

**What this phase does not attempt to verify**: real notification delivery on Linux/macOS (this environment is Windows-only) — same situation `step15.md`/`step17.md`'s CI-matrix verification handled for other cross-platform claims. Flagged in the Verification Plan below as something to confirm for real once CI can run it, not assumed from documentation alone.

---

## Decisions

### 1. Library: `notify-rust`

Per the feasibility check above — verified compiling, no new C-compiler dependency, actively maintained, the de facto standard Rust crate for this.

### 2. Which events trigger a notification

**Confirmed** (asked directly): reuse the existing frozen `Event` enum (`development.md` §3 — no new variants), subscribing a new `EventHandler` to exactly these four:
- `Event::SlackMessageReceived`
- `Event::GitHubPRCreated`
- `Event::CalendarReminderTriggered`
- `Event::SystemAlert` (covers Pomodoro session-end, `step18.md`)

**Deliberately excluded**: `SlackPresenceChanged`/`IntegrationStatusChanged` (too frequent/low-signal — a teammate going Away or a reconnect attempt isn't something worth interrupting another app for) and `PluginCustomEvent` (no plugin currently uses it for anything notification-worthy; add if a real plugin need shows up, not speculatively).

### 3. Failure handling: log and continue, never crash the app over a notification

**Confirmed** (not separately asked — low-stakes, entailed by the feature existing at all): if the OS notification backend is unavailable (e.g. no D-Bus session on a headless Linux box, notification permission denied on macOS), `notify-rust`'s `Notification::show()` returns `Err` — logged via `tracing::warn!` and dropped, exactly like this project already treats other best-effort side channels (the Pomodoro terminal bell has no error handling either, since a bell failing to sound isn't fatal to anything). The app's core functionality must never depend on notifications succeeding.

### 4. No click-through actions in this phase

**Confirmed** (not separately asked — low-stakes): notifications are fire-and-forget, no "click to open" action wired to bring the terminal to focus. `notify-rust` supports click actions on Linux via D-Bus, but not portably across all three Tier-1 platforms, and "focus a specific terminal window" isn't something any of the three OSes expose uniformly to a background process. Revisit if a real cross-platform mechanism surfaces; not worth blocking this phase on.

### 5. No mute/quiet-hours control in this phase

**Confirmed** (not separately asked — matches established project discipline): notifications are always on while the app is running — no config toggle, no `/notify off` command, no quiet-hours schedule. Matches this project's "don't build for hypothetical needs" discipline; add a mute control if it turns out to be needed in practice, not preemptively.

---

## Proposed Changes

#### [MODIFY] `Cargo.toml` (workspace), `crates/notifications/Cargo.toml`, `crates/app/Cargo.toml`
Add `notify-rust` as a workspace dependency.

#### [NEW] `crates/notifications` crate
A new crate, not a module inside the `app` binary — mirrors `crates/ipc`'s precedent (`step15.md` Decision 2: "mirroring how `plugin-host`/`plugin-sdk` each got their own crate ... rather than living inside `app`"). `DesktopNotifier` implementing `events::EventHandler`, matching on the four `Event` variants from Decision 2, building a title/body from each `NotificationItem`'s fields (or `SystemAlert`'s `String` directly), calling `notify_rust::Notification::new()...show()`, logging failures per Decision 3.

#### [MODIFY] `crates/app/src/main.rs`
Construct the `DesktopNotifier` and `register_handler` it on the existing `event_dispatcher`, alongside `projector`/`plugin_host` (same pattern, no new wiring shape).

---

## Verification Plan

- Unit tests can't assert a real OS toast appeared — instead, test the *mapping* logic (`Event` → notification title/body text) directly against a fake/injectable notifier port, the same seam pattern `IpcStatusProvider`/`PluginPresenceProvider` already established for "can't unit-test the real external system, so unit-test against an injected trait instead."
- Manual verification on this (Windows) machine: run the real app, trigger each of the four events for real (Pomodoro completing is the easiest to trigger on demand via `/pomodoro start 1 1`), confirm an actual Windows toast notification appears.
- **Explicitly deferred to real CI**: Linux/macOS notification delivery is not verifiable in this environment. Flag in Implementation Notes as unverified-by-this-session, to be confirmed for real the next time CI runs (matching `step17.md`/`platform-support.md`'s precedent for claims that could only be checked on the actual 3-OS matrix).
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

Shipped essentially as designed, as a new `crates/notifications` crate (Decision 2's four-event scope, Decision 3's log-and-continue failure handling, and Decisions 4/5's deliberate non-scope all held with no changes during implementation).

**`notification_for_event` kept pure and private, tested directly.** Rather than mocking `notify-rust` itself for tests (its `Notification::show()` talks to a real OS API — nothing to inject/fake without a much bigger seam than this feature warranted), the `Event` → `(title, body)` mapping is a plain private function, unit-tested directly via `use super::*` from the same module — 7 tests cover all four notifying variants plus all three excluded variants (the "excluded" tests are the real regression guard for Decision 2's scope: presence churn and connection-status flapping must never become notification spam).

**One real clippy fix**: `DesktopNotifier::new()` originally called `Self::default()` alongside a `#[derive(Default)]` on the zero-field struct — `clippy::default_constructed_unit_structs` (promoted to an error by `-D warnings`) flagged this as needless; fixed to `Self` directly and dropped the `Default` derive (a bare unit struct doesn't need one to satisfy `clippy::new_without_default`, since `new()` takes no arguments and can't meaningfully differ from a derived `Default` anyway).

**Real manual verification, not just a compiling `Ok`**: a temporary `crates/notifications/examples/notify_probe.rs` (kept — it's a reusable, ready-to-run smoke test for exactly the cross-platform verification this doc already flagged as deferred to real CI, not scratch work) calls `Notification::new()...show()` directly. Ran it for real on this Windows machine: the API returned `Ok`, and — confirmed directly by the user, who could see the actual screen — a real Windows toast notification appeared. This is the same "verify empirically, don't trust that `Ok` means what it claims" discipline `step15.md`'s IPC phase used.

**Still explicitly unverified**: Linux/macOS notification delivery (this environment is Windows-only, per the Context section's caveat) — flagged for confirmation the next time the 3-OS CI matrix runs, same treatment `platform-support.md`/`step17.md` gave other cross-platform claims that could only be partially checked in this session.

Final state: 7 new tests in `crates/notifications` (all passing), `DesktopNotifier` registered in `crates/app/src/main.rs` alongside `Projector`/`PluginHostManager` on the existing `EventDispatcher` (step "8c."). Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace` green with no regressions elsewhere.
