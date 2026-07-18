# Terminal Workspace Architecture Portal

Welcome to the central portal for the Terminal-first Developer Workspace. This document structures the complete specification suite, establishes reading hierarchies, and declares the **Architecture Freeze v1** guidelines.

`docs/` is organized into six numbered folders, one per category below, so the reading order in §1 matches the physical layout on disk — `01-product/` through `06-development/`. `decisions/` (ADRs) lives under `06-development/`, and per-integration specs (`slack.md`, `github.md`, etc.) live under `04-extensions/integrations/`.

---

## 1. Document Reading Hierarchy & Order

To understand the system design, we recommend reading the specifications in the following order:

```text
[1. Product]                  [2. Architecture]              [3. Domain Contexts]
01-product/        ------>    02-architecture/     ------>    03-domain/
  product-requirements.md       architecture.md                 bounded-context.md
  user-flows.md                 command-model.md                domain-model.md
  screen-spec.md                events.md                       workspace-state.md
  vision.md                     ui.md / keyboard.md              scheduler.md
  roadmap.md                    commands.md / theme.md           assistant.md / api.md
  features.md
                                                                           |
                                                                           v
[6. Development]     <------   [5. Operations]        <-----   [4. Extensions & Integrations]
06-development/                05-operations/                  04-extensions/
  development.md                 logging.md / metrics.md          plugins.md / plugin-lifecycle.md
  platform-support.md            error-catalog.md                 plugin-sdk.md / capability-system.md
  decisions/ (ADRs)               / error-handling.md              permission-model.md / security.md
                                  migration.md / versioning.md     integration-contract.md
                                  storage.md / configuration.md    notification-pipeline.md
                                  testing.md / testing-strategy.md state-machine.md
                                  performance.md                   integrations/ (Slack, GitHub, ...)
```

---

## 2. Directory Index

### 📦 1. Product & UI Specifications
- **[Vision (vision.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/01-product/vision.md)**: One-page goal statement, core principles, and explicit non-goals. Read this first.
- **[Product Requirements (product-requirements.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/01-product/product-requirements.md)**: Product goals, MVP scopes, and success criteria.
- **[User Flows (user-flows.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/01-product/user-flows.md)**: Interactive terminal user navigation pathways.
- **[Screen Specification (screen-spec.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/01-product/screen-spec.md)**: ASCII layout specifications and TUI window boundaries.
- **[Features (features.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/01-product/features.md)**: Early feature brainstorm (notifications, presence, dashboard, automation) — see "Early Draft Documents" note at the end of this file.
- **[Roadmap (roadmap.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/01-product/roadmap.md)**: v0.1–v1.0 milestone sequence.

### 🏛️ 2. Architectural Blueprint
- **[System Architecture (architecture.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/02-architecture/architecture.md)**: Structural layers, core schemas, and Tokio task layouts.
- **[Command Model (command-model.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/02-architecture/command-model.md)**: CQRS flow, command registration, and read-model projection.
- **[Event System (events.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/02-architecture/events.md)**: Strongly-typed event enum declarations and routing logic.
- **[TUI Docking (ui.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/02-architecture/ui.md)**: Dynamic split panel docking system.
- **[Keyboard Bindings (keyboard.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/02-architecture/keyboard.md)**: Modal key mapping and conflict resolution pipelines.
- **[CLI Command List (commands.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/02-architecture/commands.md)**: Concrete command names (`status away`, `team watch`, ...) — the actual CLI surface `command-model.md`'s `Command` enum dispatches.
- **[Themes (theme.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/02-architecture/theme.md)**: Valid `core.theme` values (see `configuration.md`).

### 🌐 3. Domain & Bounded Contexts
- **[Bounded Contexts (bounded-context.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/03-domain/bounded-context.md)**: DDD context maps, boundaries, and dependencies.
- **[Domain Model (domain-model.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/03-domain/domain-model.md)**: Entities, aggregate roots, and repository traits.
- **[Workspace State (workspace-state.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/03-domain/workspace-state.md)**: Context governing dynamic UI focus and active layouts.
- **[Scheduler Domain (scheduler.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/03-domain/scheduler.md)**: Logic for reminders, calendars, meetings, and timers.
- **[AI Assistant Domain (assistant.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/03-domain/assistant.md)**: AI context boundaries, vector stores, and prompt routing.
- **[Internal APIs (api.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/03-domain/api.md)**: Platform traits and registry APIs.

### 🔌 4. Extension Specifications
- **[Plugin Specifications (plugins.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/04-extensions/plugins.md)**: Plugin engine overview and host boundaries.
- **[Plugin Lifecycle (plugin-lifecycle.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/04-extensions/plugin-lifecycle.md)**: State machine, Wasm hooks, and runtime bounds.
- **[Plugin SDK (plugin-sdk.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/04-extensions/plugin-sdk.md)**: WASM WIT component definition specifications.
- **[Capability System (capability-system.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/04-extensions/capability-system.md)**: Permission check protocols.
- **[Permission Model (permission-model.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/04-extensions/permission-model.md)**: Capability grant/revoke rules, complementing `capability-system.md`.
- **[Security Architecture (security.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/04-extensions/security.md)**: Credential storage, log scrubbing, transport security, and audit logging.
- **[Integration Contract (integration-contract.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/04-extensions/integration-contract.md)**: Standard adapter protocols for external services.
- **[Notification Pipeline (notification-pipeline.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/04-extensions/notification-pipeline.md)**: Rate limiting and rule engine specs.
- **[Adapter State Machine (state-machine.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/04-extensions/state-machine.md)**: Integration adapter connection lifecycle (Disconnected → Connecting → Connected → Reconnecting).
- **[Per-Integration Specs (integrations/)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/04-extensions/integrations/)**: `slack.md`, `github.md`, `gmail.md`, `calendar.md`, `jira.md`.

### ⚙️ 5. Quality & Operations
- **[Logging Standards (logging.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/logging.md)**: Correlation ID rules, audit scopes, and tracing formats.
- **[System Telemetry (metrics.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/metrics.md)**: Profiling, event drops, and latency tracking.
- **[Performance Targets (performance.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/performance.md)**: Startup/memory/latency budgets. ⚠️ Numbers here (startup <200ms, memory <100MB) don't match `product-requirements.md` §5 (startup <150ms, memory <50MB) — pre-existing conflict, not introduced by this reorganization; reconcile before relying on either as the binding target.
- **[Error Catalog (error-catalog.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/error-catalog.md)**: Standardized codes, severity mappings, and recovery policies.
- **[Error Handling Patterns (error-handling.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/error-handling.md)**: Early note on retry/backoff/circuit-breaker patterns — mostly superseded by `error-catalog.md` and `02-architecture/events.md`'s implemented Retry Policy.
- **[Versioning Policy (versioning.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/versioning.md)**: SemVer policies and deprecation pathways.
- **[Storage Migration (migration.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/migration.md)**: Database evolution paths and rollback scripts.
- **[Storage & Persistence (storage.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/storage.md)**: SQLite schema, directory layout, and migration execution flow.
- **[Configuration (configuration.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/configuration.md)**: `config.toml` schema and the layered `ConfigBuilder` (Default → File → Env → CLI).
- **[Test Strategy (testing.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/testing.md)**: Test architecture layers and mocking conventions — overlaps with `testing-strategy.md` below; both are current, neither is deprecated.
- **[Testing Strategy (testing-strategy.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/05-operations/testing-strategy.md)**: Automated snapshot, unit, integration, and chaos testing specs.

### 📝 6. Development Rules & Decisions
- **[Development Guidelines (development.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/development.md)**: Code formatting, commit formats, and ADR change workflows.
- **[Platform Support Policy (platform-support.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/platform-support.md)**: Tier 1/experimental platform matrix, toolchain rationale (MSVC vs GNU on Windows), and CI enforcement.
- **[Architectural Decision Records (ADRs)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/)**:
  1. [ADR 0001: Language & Core Architecture](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0001-architecture.md)
  2. [ADR 0002: WebAssembly Component Model Sandbox](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0002-plugin-system.md)
  3. [ADR 0003: Asynchronous Event Bus](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0003-event-bus.md)
  4. [ADR 0004: Relational SQLite Storage Selection](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0004-storage.md)
  5. [ADR 0005: Technology Stack & Impact Analysis](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0005-technology-stack.md)
  6. [ADR 0006: SecretProvider Chain](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0006-secret-provider.md)
  7. [ADR 0007: CQRS and Read Projections](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0007-cqrs.md)
  8. [ADR 0008: Domain Bounded Contexts Boundaries](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0008-ddd.md)
  9. [ADR 0009: Plugin SDK WIT Bindings](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0009-plugin-sdk.md)
  10. [ADR 0010: Core/Plugin Unified Registries](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0010-registry-pattern.md)
  11. [ADR 0011: Notification Pipeline Stages](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0011-notification-pipeline.md)
  12. [ADR 0012: UI View Docking System](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0012-docking-system.md)
  13. [ADR 0013: Rule Engine Integration](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0013-rule-engine.md)
  14. [ADR 0014: Storage Engine Reconsideration (SQLite → redb)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0014-storage-engine-reconsideration.md) — supersedes ADR-0004's engine choice.
  15. [ADR 0015: Release Packaging via cargo-dist](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0015-release-packaging.md)
  16. [ADR 0016: Extending `enum Event` with `IntegrationStatusChanged`](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/06-development/decisions/0016-event-enum-extension.md)

---

## 3. Note on Early Draft Documents

A handful of files predate the fuller specs above and are much terser (`vision.md`, `roadmap.md`, `features.md`, `commands.md`, `theme.md`, `state-machine.md`, `performance.md`, `error-handling.md`). They weren't indexed in this README before this reorganization pass. Most still carry unique information not written anywhere else (e.g. `commands.md`'s concrete CLI command list, `theme.md`'s valid theme names) and have been filed alongside their closest topical sibling rather than discarded. `error-handling.md` is the one clear exception — its content is now superseded by `error-catalog.md` and the implemented Retry Policy in `02-architecture/events.md`. If you're relying on one of these files for a concrete value, cross-check it against the fuller doc in the same folder first — `performance.md` vs. `product-requirements.md`'s conflicting startup/memory targets (noted in §2.5 above) is a good example of why.
