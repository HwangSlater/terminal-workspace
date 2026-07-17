# Terminal Workspace

A terminal-first developer workspace: Slack, GitHub, Gmail, Google Calendar, Jira, and CI/CD in one place, without leaving the terminal.

Local First. Zero Configuration. Cross-platform (Windows, macOS, Linux) with no OS treated as second-class — see [`docs/06-development/platform-support.md`](docs/06-development/platform-support.md).

---

## Quick Start

**1. Install Rust**, if you haven't already: <https://rustup.rs> — that's the only prerequisite. No C compiler, no database server, no extra toolchain (storage is pure-Rust `redb`; see [ADR-0014](docs/06-development/decisions/0014-storage-engine-reconsideration.md)).

**2. Run it:**

```sh
cargo run -p app
```

No config file to write by hand — first run bootstraps `config.toml` and the local database automatically (see [`docs/05-operations/configuration.md`](docs/05-operations/configuration.md) §4).

Optional: `scripts/setup.ps1` (Windows) / `scripts/setup.sh` (Linux/macOS) is a one-command sanity check (confirms `rustup` is present, runs `cargo check --workspace`) if you want a clear pass/fail before diving in.

---

## Project Status

This project is under active architecture-first development. Phase 2 (Core Infrastructure: Event Bus, Registries, Config, Secrets, Logging) and Phase 3 (Storage + CQRS write path) are implemented — see [`step2.md`](step2.md) and [`step3.md`](step3.md) for what each phase covers and why.

## Documentation

The full architecture, design decisions, and specifications live in [`docs/`](docs/README.md) — start there for anything beyond "how do I run this."

## Development

- `cargo check --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo fmt --check` / `cargo test --workspace`
- See [`docs/06-development/development.md`](docs/06-development/development.md) for code style, the feature-change process, and the Architecture Freeze v1 rules this codebase follows.
