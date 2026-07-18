# Implementation Plan - Phase 2: Core Infrastructure

Having verified the dependency flow (unidirectional dependencies, no cycles) and declared the Architecture Freeze v1, we proceed to **Phase 2: Core Infrastructure**. This phase implements the foundational sub-systems required for platform communication, security, telemetry, and settings loading.

---

## User Review Required

We are implementing the central loop mechanisms of the Event Bus and thread-safe registry containers.

> [!IMPORTANT]
> **Core Sub-system Implementations**:
> 1. **InProcessEventBus**: Leverages Tokio's broadcast channels for thread-safe event publishing and handler dispatching.
> 2. **Thread-Safe Registries**: Creates `InMemory` adapters for the Command, UI, and Service registries using `std::sync::Arc` and `tokio::sync::RwLock`.
> 3. **Config parser**: Connects the `toml` parser and validates core timing ranges.
> 4. **SecretProvider Chain**: Fleshes out Env and Keyring lookup flows wrapping secrets in `secrecy::SecretString`.
> 5. **Logging Spans**: configures tracing with formatted outputs.

---

## Proposed Changes

We will modify the stubs inside the following crates:

### Crates Implementation

#### [MODIFY] [crates/events/src/lib.rs](file:///c:/Users/pc/Desktop/terminal-workspace-docs/crates/events/src/lib.rs)
- Implement `InProcessEventBus` using `tokio::sync::broadcast` and `tokio::sync::RwLock` for registering `EventHandler` lists.

#### [MODIFY] [crates/registry/src/lib.rs](file:///c:/Users/pc/Desktop/terminal-workspace-docs/crates/registry/src/lib.rs)
- Implement `InMemoryCommandRegistry`, `InMemoryUiRegistry`, and `InMemoryServiceRegistry` utilizing thread-safe async-locks.

#### [MODIFY] [crates/config/src/lib.rs](file:///c:/Users/pc/Desktop/terminal-workspace-docs/crates/config/src/lib.rs)
- Complete TOML parsing logic validations and configuration fallback mapping.

#### [MODIFY] [crates/secrets/src/lib.rs](file:///c:/Users/pc/Desktop/terminal-workspace-docs/crates/secrets/src/lib.rs)
- Complete the provider resolution loop inside `SecretProviderChain`.
- Implement `EnvProvider` using `std::env::var`.

#### [MODIFY] [crates/logging/src/lib.rs](file:///c:/Users/pc/Desktop/terminal-workspace-docs/crates/logging/src/lib.rs)
- Set up tracing subscriber layers.

---

## Verification Plan

We will verify Phase 2 features using automated Rust tests:
- **Event Bus Tests**: Verify that subscribing handlers receive published Events on correct threads.
- **Registry Concurrency Tests**: Ensure that multiple async tasks can register and query commands/services concurrently without causing deadlock conditions.
- **Config Validation Tests**: Assert that config files with `< 16ms` refresh rates return configuration validation errors.

---

## Revision After Feedback (`step2_feedback.md`)

Reviewer feedback approved the plan ("Proceed 승인") but asked for six structural refinements. On inspection, most of the crate code already matched the intended design (EventBus/EventDispatcher were already split; the registries already exposed only a minimal surface; `SecretProviderChain` was already `Vec<Box<dyn SecretProvider>>`). The remaining gaps closed in this revision:

1. **Event Bus / Dispatcher**: no code change needed — documented the existing split in `docs/02-architecture/events.md` and moved priority-routing/retry/DLQ to an explicit Phase 3 note (ADR-0003 amended).
2. **Registry**: confirmed the minimal surface already satisfies the feedback; **no trait changes**, since `docs/06-development/development.md` §3 (Architecture Freeze v1) locks `CommandRegistry`/`ServiceRegistry`/`UiRegistry` signatures without a new ADR. Documented this decision in ADR-0010.
3. **Config**: added a layered `ConfigBuilder` (Default → File → Env → CLI) to `crates/config`, per `docs/05-operations/configuration.md` §3. CLI parsing is a small hand-rolled scanner, not a new `clap` dependency, given the current 4-flag surface.
4. **SecretProvider**: added `SecretProviderChain::default_chain()` to assemble the ADR-0006 canonical order (`Env → Keyring → EncryptedFile`); the chain itself was already extensible.
5. **Logging**: added a `spans` module defining the `Application > Command / Event / Integration / Plugin` hierarchy up front, plus a `RedactingLayer` closing a pre-existing, previously-unimplemented requirement from `docs/04-extensions/security.md` §2 / `docs/05-operations/logging.md` §3.
6. **Zero Configuration UX**: documented and wired the first-run flow (`$ tw` with no setup) in `docs/05-operations/configuration.md` §4; fixed a bug in `crates/app/src/main.rs` where the bootstrap TOML nested `[integrations.slack]` instead of the flat schema `AppConfig` actually parses.
7. **Vertical Slice Test**: extended `crates/app/tests/core_infra_test.rs` to also build `AppConfig` via `ConfigBuilder` and initialize logging, so the test proves Config + Logging + EventBus + Registry compose end-to-end without any real integration — without pulling forward the CQRS `CommandDispatcher` work, which stays out of scope for Phase 2.
