# Changelog

All notable changes to Terminal Workspace are documented here. Full development history — what was built when and why — lives in [`docs/07-implementation-log/`](docs/07-implementation-log/).

## [1.0.0] - 2026-07-19

First public release.

### Added

- **Interactive TUI shell** — Vim-inspired modal keyboard control, responsive layout down to an 80×24 terminal, an always-visible Team roster in the header plus Notification/Calendar body panels.
- **Slack integration** — Bot Token setup in-app, channel/user picker, `/send`/presence commands with autocomplete, live connection status.
- **GitHub integration** — Personal Access Token setup, repository picker, open PR notifications.
- **Calendar integration** — private iCal feed URL (no OAuth), upcoming/recurring event reminders.
- **Pomodoro timer** — `/pomodoro start|pause|reset`, live countdown in the header, terminal bell + auto mode-switch on session end.
- **Log viewer** — `Ctrl+C` opens a scrollback overlay of the app's own logs, colored by level, secrets automatically redacted; also written to a rotating file on disk.
- **Desktop notifications** — real OS toast notifications for Slack DMs, GitHub PR review requests, Calendar reminders, and Pomodoro session-end, so you don't have to be looking at the app to notice.
- **Terminal tab/window title badge** — shows the unread notification count, visible from a different tab without switching to it.
- **WebAssembly plugin runtime** (experimental, opt-in) — sandboxed with CPU/memory limits; a misbehaving plugin can't affect the rest of the workspace.
- **Daemon mode & local CLI** — the running TUI instance is reachable from another terminal via `termws slack-send`/`set-presence`/`status`.
- **Zero Setup** — no database server, no config file to hand-write; everything needed beyond `rustup` is either pure Rust or already on the OS (the one exception: the optional plugin runtime's WASM sandbox needs a C compiler to build on Windows/Linux, see the README).
- **Cross-platform** — Windows, macOS, and Linux all built and tested as Tier 1, not an afterthought.

### Known limitations

- Windows/macOS installers are **unsigned** — first run will show an OS warning (Windows SmartScreen / macOS Gatekeeper) that has to be clicked through once. Accepted for this release rather than budgeting for a code-signing certificate; revisit if it becomes a real adoption blocker.
- Desktop notification delivery has only been verified on Windows in this project's development environment; Linux/macOS delivery is expected to work (same cross-platform library, feasibility-checked for each OS) but not yet confirmed against a real run.
- Gmail, Jira, and CI/CD integrations, and an AI assistant panel, are not part of this release. Gmail/Jira/CI/CD remain on the roadmap; the AI assistant was designed in detail and then deliberately not pursued (see `docs/07-implementation-log/step23.md`).
