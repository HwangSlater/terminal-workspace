# Product Requirements Document (PRD)

## 1. Project Objectives
Provide an extensible, terminal-first developer workspace platform that aggregates communication, development, scheduling, and automation tools, allowing developers to perform daily activities without switching contexts to GUI programs.

---

## 2. Platform Portability & Usability Principles

**User Experience and Contributor Experience are explicitly different audiences and are not held to the same bar.** Conflating them is how a build-time requirement (a C toolchain, needed only to compile `rusqlite`'s bundled SQLite from source) could be mistaken for a violation of "Zero Setup" — it isn't, as long as it never reaches an end user. See `docs/06-development/platform-support.md` for how the two are kept separate in practice.

### 2.1 User Experience Principle: Zero Setup

> A user should never have to study the platform to use it. (`vision.md`)

**One Command Startup**: a user runs `tw` (or the platform binary) and is immediately working. No compiler, no database server, no config file to hand-author, no environment variables to set. This is achieved by:

1. **Zero-Config Out-of-the-Box**: The application requires zero configuration files, environment variables, or background services to launch. On its first execution, it bootstraps default layouts, in-memory caches, and demo contexts immediately.
2. **OS Independence**: The binary holds zero system-level dynamic library linkages (storage is `redb`, a pure-Rust embedded store with no native dependency — see ADR-0014 — and keyring checks fail gracefully). It runs as a self-contained single executable.
3. **Execution Simplicity**: Runs simply via `terminal-workspace` command. No external database installations (like Redis or MySQL) or message brokers are required.
4. **Prebuilt release binaries**: the mechanism now exists (`cargo-dist`, ADR-0015, `step4.md` Phase 4) — a user never installs Rust at all, they download a binary/installer for their OS and run it. Currently validated via pre-release tags only, against the pre-TUI skeleton (`step4.md`'s sequencing decision); the real public release, and this item's completion, waits for Phase 5 (TUI). Until then, the remaining gap is purely "has Rust installed and runs `cargo run`" vs. "downloads a binary" — not a toolchain-setup gap, since building from source no longer requires a C toolchain either (§2.2).

### 2.2 Contributor Experience Principle: One-Time Setup

This section originally documented a real cost: building from source required installing a C toolchain, because `rusqlite`'s `bundled` SQLite compiled from C (ADR-0004). Living with that cost — including a contributor onboarding session that lost hours to a stuck `winget`/Visual Studio Build Tools install — was the direct trigger for ADR-0014, which replaced the storage engine with `redb` (pure Rust, no C compiler) rather than continuing to manage around the requirement.

**Current state**: building from source requires only `rustup` (a Rust toolchain) — nothing else, on any of the three Tier 1 platforms (`docs/06-development/platform-support.md`). `scripts/setup.ps1` / `scripts/setup.sh` are a one-command verification step (confirm `rustup` is present, run `cargo check --workspace`), not an installer.

This means the Contributor Experience gap this section exists to manage has nearly collapsed into the User Experience principle above — the remaining difference is "has Rust source and runs `cargo run`" vs. "downloads a prebuilt binary and runs it" (§2.1 item 4), not a toolchain install. Kept as its own principle because that gap (source vs. prebuilt binary) is still real until v1.0.0 ships release artifacts — but there is no "one-time setup cost" left to actually describe today.

---

## 3. Scenarios & CLI Integrations
- **Daily Standup Preparation**: Developer opens workspace, reviews unread Slack mentions, checks calendar for the standup link, and sets their presence.
- **Vim / Tmux Shell IPC**: While editing code inside Vim, a developer executes `:!termws slack-send "@bob" "Here is the patch"`. The local IPC client pushes this command to the running background workspace daemon via Unix Domain Sockets (Linux/macOS) or Named Pipes (Windows), avoiding full app swaps.
- **AI Automation**: Developer executes `/ask how to resolve build error`, the AI inspects the context buffer and recommends a fix.

---

## 4. Release Scopes

### MVP (v0.1.0)
- TUI Docking layout shell.
- Slack integration (Message read, Presence sync).
- System notification broker (without complex rules).
- Core Command/Service registries.
- **Zero-Config Startup and local file fallback secrets**.

### v1.0.0 (Release)
- Full Bounded Context isolation.
- WASM Sandboxed Plugin runtime.
- Multi-integration adapters (Slack, GitHub, Calendar).
- Local `redb` embedded key-value caching (ADR-0014; superseded the original SQLite plan).
- **Daemon mode & Local CLI Socket IPC**.
- **Public release with prebuilt binaries** for all three Tier 1 platforms (`docs/06-development/platform-support.md`): Windows (`.msi`), macOS (tarball, `.dmg` planned), Linux (tarball, shell installer). The `cargo-dist` pipeline itself (ADR-0015, `step4.md` Phase 4) already exists and is validated via pre-release tags against the pre-TUI skeleton — what's left for v1.0.0 is the actual public announcement, gated on Phase 5 (TUI) landing first so there's something worth downloading.

---

## 5. Success Metrics
- **Startup Time**: Cold start to fully loaded widgets < 150ms.
- **Resource Usage**: Base idle memory < 50MB.
- **Execution Overhead**: Running `termws --help` or launching stubs requires zero prerequisite setup.
