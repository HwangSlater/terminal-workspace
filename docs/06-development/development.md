# Development Guidelines

This document details the code style, linting rules, naming conventions, and branch management strategies required to maintain the long-term quality of the Terminal Workspace. It also enforces the **Architecture Freeze v1** specifications and the **Document Change Procedures**.

---

## 1. Document Precedence Rules (문서 우선순위)

When specifications or designs conflict across files, resolutions are strictly guided by the following order of precedence:

| Precedence | Document Category / Target File | Action on Conflict |
| :---: | :--- | :--- |
| **1** | [Product Requirements (product-requirements.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/01-product/product-requirements.md) | Standard for functional scope and goals. |
| **2** | [System Architecture (architecture.md)](file:///c:/Users/pc/Desktop/terminal-workspace-docs/docs/02-architecture/architecture.md) | Ultimate system layout and layer boundary authority. |
| **3** | Architectural Decision Records (ADRs inside `docs/06-development/decisions/`) | Enforces localized component choices. |
| **4** | Domain Context & Model Specifications (`bounded-context.md`, `domain-model.md`) | Aggregates and entity rules authority. |
| **5** | Interface Specifications (`api.md`, `plugin-sdk.md`, `ui.md`, `keyboard.md`) | Defines traits, layouts, and input hooks. |
| **6** | Implementation Code / Local scratch scripts | Source code must adapt to documentation, not vice versa. |

---

## 2. Feature Implementation Change Procedures (변경 절차)

Every new feature or bug fix must undergo a systematic lifecycle before any production code is committed.

```text
  [1. Requirement Review]
            │
            ▼
  [2. Architecture Impact Analysis] ──(Requires structural shift?)──Yes──> [3. Create ADR]
            │                                                                   │
            No                                                                  v
            │ <─────────────────────────────────────────────────────────────────┘
            ▼
  [4. Document Refactoring] (Modify docs/ specs matching precedence rules)
            │
            ▼
  [5. Write Automated Tests] (Unit/Integration tests outlining target interfaces)
            │
            ▼
  [6. Code Implementation] (Fulfill the spec until tests pass)
            │
            ▼
  [7. Document Validation] (Confirm code exactly matches markdown linkages)
```

1. **Requirement**: Clarify "what to build" using `product-requirements.md`.
2. **Architecture Impact**: Inspect dependencies across Bounded Contexts.
3. **ADR Assessment**: If introducing new frameworks, modifying repositories, or event schemas, write a new sequential ADR.
4. **Document Revision**: Update relevant specification markdown files *prior* to coding.
5. **Test First**: Write skeleton test cases (`cargo test` assertions or Ratatui `TestBackend` buffer layout snapshots).
6. **Implementation**: Code the target adapter or service context.
7. **Verify**: Run doc-link check loops.

---

## 3. Architecture Freeze v1 Guidelines (설계 동결 기준)

We declare **Architecture Freeze v1** active. The following baseline interfaces and boundaries are frozen and cannot be modified without an approved Architectural Decision Record (ADR):

- **Bounded Context Boundaries**: The 8 contexts (`Workspace`, `Notification`, `Presence`, `Plugin`, `Task`, `Integration`, `Scheduler`, `Assistant`) cannot be merged or have their dependency flow modified.
- **Registry Interfaces**: Signature modifications to `CommandRegistry`, `ServiceRegistry`, and `UiRegistry` are blocked.
- **Repository Contracts**: Trait definition contracts in `domain-model.md` (e.g., `NotificationRepository`) are locked.
- **Event Contracts**: The core strongly typed `enum Event` mapping cannot be altered.
- **Plugin SDK WIT Contracts**: `plugin-sdk.wit` Component model specifications are frozen.

---

## 4. Code Style & Linters

- **Formatter (`rustfmt`)**: Run `cargo fmt --check` in pre-commit hooks.
- **Linter (`clippy`)**: Run `cargo clippy --all-targets -- -D warnings`.
- **Platforms**: `cargo fmt --check`, `cargo check`, `cargo clippy -D warnings`, and `cargo test` must all pass on every Tier 1 target (Windows/MSVC, Linux/GNU, macOS) — see `docs/06-development/platform-support.md` for the tier table and rationale, enforced by `.github/workflows/ci.yml`'s 3-OS matrix.
- **Conventions**:
  - Types / Structs / Traits / Enums: UpperCamelCase
  - Functions / Modules / Variables: snake_case
  - Constants: SCREAMING_SNAKE_CASE
  - Errors: End with `Error` suffix.
- **Conventional Commits**: Commit messages must adhere to `feat(<scope>): <subject>` format.
