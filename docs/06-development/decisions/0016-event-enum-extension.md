# ADR 0016: Extending `enum Event` with `IntegrationStatusChanged`

## Context

`docs/06-development/development.md` §3 (Architecture Freeze v1) locks "the core strongly typed `enum Event` mapping" — changes require an ADR, not a silent edit. `step9.md` (Phase 9) needs the render loop to know when Slack's connection status changes, live, without waiting for the user's next keypress. The status transition already happens inside `crates/integration::SlackPoller::run_loop`; the only way for `crates/ui` to observe it without polling is a new `Event` variant broadcast over the existing `EventBus`.

## Decision

Add one variant:

```rust
pub enum Event {
    // ...existing variants, unchanged...

    /// An integration adapter's connection status changed.
    IntegrationStatusChanged {
        source: IntegrationSource,
        status: IntegrationConnectionStatus,
    },
}
```

`IntegrationConnectionStatus` is a **new, separate enum defined in `crates/events`** (`Disconnected`/`Connecting`/`Connected`/`Reconnecting`/`Failed(String)`), not a re-export of `integration::ConnectionStatus`. `crates/events` cannot depend on `crates/integration`: `integration` already depends on `events` for `EventBus`/`Event`, so the reverse direction would be circular. `crates/integration` maps its own `ConnectionStatus` into this type at the point of publishing — the same "adapter translates its own wire/internal types into shared domain-adjacent types before broadcasting" pattern already used for Slack API JSON → `NotificationItem`/`MemberPresence` (ADR-0008's "Integration adapters must map external data structures into pure Domain objects before broadcasting to the Event Bus").

Existing variants are untouched. `Event::SystemAlert` (raised once, on the transition into `Failed`, per `integration-contract.md` §2.1) is not replaced or duplicated by this — `IntegrationStatusChanged` fires on *every* transition (including the routine `Connected`→`Reconnecting` ones `SystemAlert` doesn't cover) and is consumed differently (by the UI directly, not through the DLQ/retry path).

## Alternatives Considered

### Reuse `Event::SystemAlert(String)` for status changes too
- **Pros**: no ADR needed, no new variant.
- **Cons**: conflates two different signals (an alert worth surfacing prominently vs. a routine status transition the header just reflects) into one loosely-typed string the UI would have to parse to recover the actual status. Already-existing `SystemAlert` consumers (the DLQ retry-exhaustion path) have no reason to also fire on every `Reconnecting` blip.

### Poll `SlackAdapter::health_check()` on every redraw instead of pushing an event
- **Pros**: no `Event` enum change, no ADR, no new dependency from `crates/ui` on `crates/events`.
- **Cons**: this was the phase's original, smaller-scoped design (see `step9.md`'s Decision 3, Option A) — correct as of the last keypress/resize, not actually live. Superseded once the decision was made to fix the deeper issue (background changes not triggering a redraw at all) rather than just the status-badge symptom of it.

## Consequences

- `crates/ui` gains a direct dependency on `crates/events` (previously only reachable transitively through `crates/commands`) to receive `Event` off the bus and read `IntegrationConnectionStatus`.
- `crates/commands::Projector`'s `EventHandler::handle` match must add an arm for the new variant (exhaustive match) — a no-op, since this event's job is done by `crates/ui`'s own direct bus subscription, not the read model.
- Any future adapter (GitHub, Calendar) reports its status through the same variant, keyed by `source: IntegrationSource` — no new variant needed per integration.
