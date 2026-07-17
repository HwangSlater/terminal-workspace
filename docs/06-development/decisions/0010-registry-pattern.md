# ADR 0010: Core/Plugin Unified Registries

## Context
We need to register system commands (like `/slack-send`, `/status away`), views, and services. If plugins use a different registry interface than the core platform features, we create two separate execution tracks, causing duplicate code and API inconsistency.

---

## Decision
We enforce a **Unified Registry Pattern** where both core built-in features and WASM plugins register to the same registry instances:
- `CommandRegistry`: Stores all executable CLI targets.
- `ServiceRegistry`: Connects callers to abstract services (e.g. `Notification API`).
- `UiRegistry`: Maps views and slots.

---

## Alternatives Considered

### Segregated Registries (Core vs. Plugins)
- **Pros**: Easier to lock down plugin privileges.
- **Cons**: High duplicate code. The Command Dispatcher would have to query both the "Core Command List" and the "Plugin Command List" and resolve conflicts manually. (Rejected).

---

## Consequences
- **API Parity**: Every action a plugin can perform is built on the same API contracts as the core features, simplifying platform testing.
- **Dynamic Extensibility**: Core features can be easily refactored out into standard plugins later without rewriting interface layers.

---

## Amendment (Phase 2 Implementation Note)

`docs/06-development/development.md` §3 (Architecture Freeze v1) locks the `CommandRegistry`, `ServiceRegistry`, and `UiRegistry` trait signatures — they cannot change without a new ADR. During Phase 2 review (`step2_feedback.md`) we checked the frozen surface against the recommended minimal registry API (register / get / contains / remove / iter) and found it already satisfied without changes:

| Recommended | `CommandRegistry` | `UiRegistry` | `ServiceRegistry` |
| :--- | :--- | :--- | :--- |
| register | `register_command` | `register_panel` | `register_service` |
| get | — (not needed yet) | `list_slot_panels` | `get_service` |
| contains | — (not needed yet) | — (not needed yet) | — (not needed yet) |
| remove | `remove_command` | `unregister_panel` | — (not needed yet) |
| iter | `list_commands` | `list_slot_panels` | — (not needed yet) |

No trait signatures were added or changed in Phase 2. The blank cells above are not implemented because no current caller needs them; adding a method to a frozen trait for a hypothetical future caller would be scope creep this ADR explicitly warns against ("Segregated Registries... duplicate code" was rejected for the same reason — unnecessary surface area). If a concrete consumer needs single-item lookup or existence checks later, that should land as a small follow-up ADR at that time, not now.
