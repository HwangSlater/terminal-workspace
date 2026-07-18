# ADR 0005: Core Technology Stack Selection & Impact Analysis

## Context
The Terminal Workspace requires a production-grade, highly performant, and long-term maintainable technical stack. This document details the choices, alternatives, pros/cons, and impact analysis for our stack selection.

---

## Technical Stack Table

| Tech / Library | Choice | Alternative | Pros | Cons | Replacement Cost | Impact Scope |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Language** | Rust | Go, C++ | Memory safety, zero-cost abstractions, compiled binary. | Steeper learning curve, compilation times. | Extremely High (rewrite core) | Entire project |
| **Async Runtime** | Tokio | async-std, custom event loop | Mature ecosystem, highly optimized multi-threaded executor. | Complex thread synchronization rules. | High | Core platform thread model |
| **TUI Engine** | Ratatui | Textual (Python), Bubbletea (Go) | Double buffering prevents flickering, highly composable widget API. | Complex drawing math for custom splits. | Medium | Presentation Layer only |
| **Plugin Isolation** | WASM Component Model | Native `dylib` loading, Lua scripting | Dynamic loading, multi-language, capability sandboxing. | Host-guest serialization overhead. | High | Plugin Context |
| **Serialization** | Serde | custom JSON parsing, Protobuf | Compile-time generation, ultra-fast JSON/TOML translation. | Code bloat due to macros. | Medium | Integration / Storage boundary |
| **Database** | `redb` (pure-Rust embedded KV store) | SQLite (rusqlite bundled), sled, rocksdb, raw file IO | No C compiler / build script required on any OS — `cargo build` just works. ACID transactions. See ADR-0014. | No relational queries (not needed by current access patterns — see ADR-0014). | Medium | Storage infrastructure |
| **Logging** | Tracing | env_logger, log | Structured spans, context propagation (Correlation ID). | Complex initialization logic. | Low | All layers |
| **Error Handling** | thiserror + anyhow | std::error::Error | `thiserror` for library domain errors; `anyhow` for app workflows. | Boilerplate declarations for custom error classes. | Low | All modules |
| **Local IPC** | `interprocess` (Unix Domain Sockets / Named Pipes) | Hand-rolled `tokio::net::UnixListener` + `tokio::net::windows::named_pipe`, gRPC/D-Bus | Pure Rust (verified via `cargo tree -i cc` before adopting, per ADR-0014's practice), one API across platforms instead of two bespoke code paths for Windows named-pipe security semantics. See `step15.md`. | One more dependency; less control than hand-rolling if a very specific low-level behavior were ever needed. | Medium | `crates/ipc` only |

---

## Alternatives Considered & Rejected

### Go & Bubbletea (TUI)
- Go is highly concurrent, but its Garbage Collector (GC) introduces micro-stutters during 60 FPS terminal renders. Go's plugin system is platforms-restricted (not supporting Windows), whereas our targets include cross-platform developers.

### SQLite (`rusqlite`, bundled or dynamic)
- Originally selected (see the now-superseded row above and ADR-0004) for relational query capability. Dynamic linking to OS-native `libsqlite3` was rejected outright (requires pre-installed packages on Ubuntu or DLL setup on Windows). `bundled` static compilation avoided that but introduced a C-compiler build requirement that caused real, hours-long contributor friction in practice — see ADR-0014, which reconsiders the engine choice entirely rather than picking a different SQLite linking mode.

---

## Consequences
- The choice of Rust + Tokio ensures execution speed and safety, but requires strict adherence to asynchronous code boundaries.
- `redb` (see ADR-0014) means the workspace builds from a clean machine with nothing beyond `rustup` — no bundled-C-source compile step, on any OS.
- Shifting to WASM Component Model ensures the platform is protected from buggy/malicious plugin integrations.

---

## Amendment (Platform/Toolchain Policy) — Historical

*This amendment described a real problem (documented in detail in ADR-0014's Context) that has since been resolved by changing the underlying decision, not by the mitigation described below. Kept for history.*

`bundled` rusqlite's upside (no OS dynamic-library dependency at runtime) had a corresponding cost: **build time** required a working C compiler on every developer machine and every CI runner, for every supported OS. This surfaced concretely during Phase 3 development, and again during initial contributor onboarding (zero-toolchain Windows machine, hours lost to a stuck `winget`-driven Visual Studio Build Tools install) — full account in ADR-0014.

The original mitigation here was to standardize Windows development on the MSVC target and enforce a 3-OS CI matrix — see `docs/06-development/platform-support.md`. That policy (the CI matrix, the tier table) is still in effect and still useful, but its original *motivating urgency* (everyone needs a C compiler) is gone: ADR-0014 replaced the storage engine instead of continuing to manage around the toolchain requirement.
