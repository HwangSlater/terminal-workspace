# Implementation Plan - Phase 24: Multi-Calendar Support

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-23.

## Context

`step12.md` Decision 1 deliberately scoped Calendar to one connection: the secret-iCal-URL auth model has no "list my calendars" discovery API, so a `Picker` made no sense at the time, and nothing in the product requirements asked for more than one calendar. That doc explicitly flagged this as revisitable: *"Multi-calendar support ... [is a] real capability left on the table, but nothing in the product requirements asks for [it] yet — revisitable later without an ADR."* A real user need surfaced it: wanting work and personal calendars merged into one view. `IntegrationAdapter`/`IntegrationConnector`/`Picker`/`SelectionApplier` aren't frozen (only `Event` is, per Architecture Freeze v1), so this is a normal feature addition, not an exception.

Two UX decisions confirmed directly before designing further:
1. **Each calendar gets a user-entered label** (not auto-numbered, not unlabeled) — reminders show `[회사] Design Review` instead of an unlabeled mix, so multiple calendars stay distinguishable in the Notification/Calendar panel.
2. **Removing one calendar uses a checkbox picker list** (select which to *keep*, mirroring Slack's channel picker / GitHub's repo picker exactly) — not a "wipe everything and re-add" flow.

**A real, non-obvious finding while designing this**: `Picker`/`SelectionApplier` already exist as fully generic traits (`crates/integration/src/lib.rs`, `crates/commands/src/lib.rs`) — `Picker::list_items()` and `SelectionApplier::apply()` don't care whether the "list" comes from a remote API (GitHub's repos) or purely local state (Calendar's own already-saved connections). Implementing both traits for `CalendarAdapter` reuses `crates/ui`'s existing generic picker overlay, `KeyOutcome::OpenPicker`/`SubmitSelection`, and `Command::ApplySelection{source, items}` almost entirely as-is — this phase needs one new overlay only for *adding* a calendar (label + URL), not a second one for managing/removing them.

---

## Decisions

### 1. Storage: a JSON-serialized list under a new secret key, with migration from the old single-URL key

**Confirmed**: `AdapterState.token: Option<String>` becomes `AdapterState.connections: Vec<CalendarConnection>` where `CalendarConnection { id: Uuid, label: String, url: String }` (the `Uuid` is a stable identifier for `SelectionApplier`'s `items: Vec<String>`, generated once when a calendar is added — a label alone isn't guaranteed unique, e.g. two calendars both named "회의"). The whole list is stored as one JSON string under a new secret key, `CALENDAR_CONNECTIONS` (`serde`/`serde_json` are already dependencies of `crates/integration`).

**Real backward-compatibility requirement, not optional**: anyone who already connected a calendar before this phase has a secret saved under the old `CALENDAR_ICAL_URL` key. `initialize()` must check for `CALENDAR_CONNECTIONS` first, and if absent, fall back to reading the old key and treating it as a single-entry list (auto-labeled, since no label was ever collected for it) — silently losing an existing connection on upgrade would be a real regression, not a cosmetic one.

### 2. Adding a calendar: the existing `Ctrl+L` overlay gains a second field

**The UI outcome shipped as designed; the plumbing underneath changed — see Implementation Notes.** `Ctrl+L` still opens the Calendar setup overlay, but it now collects two fields in sequence — label first (plain text, not masked — it's a display name, not a secret), then the URL (masked, as today). `Enter` on the label field (non-empty) advances to the URL field; `Enter` on the URL field (non-empty) submits both. This *adds* a new connection rather than replacing the existing set. The proposal below (a new `Command::AddCalendar { label, url }`) turned out to be unnecessary — the existing `Command::Connect`/`IntegrationConnector` plumbing was reused instead, with `token` carrying both fields (`"{label}\n{url}"`).

### 3. Removing a calendar: a new `Ctrl+K` picker overlay, reusing the existing generic Picker/SelectionApplier machinery

**Confirmed**: `CalendarAdapter` implements `Picker::list_items()` — returning the *currently connected* calendars as `PickerItem { id: connection.id.to_string(), label: connection.label.clone() }`, a local read, not a network call — and `SelectionApplier::apply(event_bus, items: Vec<String>)` — meaning "keep only the connections whose id is in `items`", i.e. unchecked entries get removed. This is the exact same shape `crates/ui` already renders for GitHub's repo picker (`j`/`k` move, `Space` toggles, `Enter` saves), reused via the already-source-generic `KeyOutcome::OpenPicker(IntegrationSource)`/`SubmitSelection(IntegrationSource, Vec<String>)` and `Command::ApplySelection`. `Ctrl+K` is a new binding (unused; `Ctrl+A` was reserved for the declined AI Assistant and stays free, but `K` was picked since GitHub's own picker binding, `Ctrl+R`, wasn't a deep mnemonic either — there's no natural free letter left that spells "remove/manage" for Calendar specifically).

### 4. Polling: one loop iterates every configured calendar each cycle

**Confirmed**: `CalendarPoller::run_loop`/`poll_once` take `Vec<CalendarConnection>` (a snapshot at `start()` time, same as today's single `url: String` is) instead of one URL, and loop over all of them per cycle, same `sync_interval_secs` for all (no per-calendar interval — nothing asks for that yet). `seen_occurrences`'s dedup key gains the connection id (`(connection_id, event_uid, timestamp)` instead of `(event_uid, timestamp)`) — cheap insurance against two different Google calendars ever coincidentally producing the same event UID. Each occurrence's `NotificationItem.title` gets the label prefixed: `format!("[{label}] {title}")` (Decision-1's confirmed disclosure need) — no change to `NotificationItem`'s shape, no new domain field.

### 5. Partial failure: one bad calendar doesn't mask the others working

**Confirmed** (not separately asked — low-stakes, but a real behavior difference worth stating): if 2 of 3 configured calendars fetch successfully and one fails, the poll cycle as a whole still reports `PollResult::Success` (so the header shows Connected, not stuck cycling toward Reconnecting/Failed over a single bad connection) — each failing connection logs its own `tracing::warn!` (same diagnostic pattern the previous phase's live bug fix established) independent of the others. Only "every configured calendar failed this cycle" counts as an overall `Failure`.

---

## Proposed Changes

#### [MODIFY] `crates/integration/src/calendar.rs`
`CalendarConnection` struct; `AdapterState.token` → `connections: Vec<CalendarConnection>`; `initialize()` gains the old-key migration (Decision 1); `IntegrationConnector::connect` stays as "replace" semantics unused by Calendar's new UI path but left intact for trait uniformity; new `add_connection`/`Picker`/`SelectionApplier` impls (Decisions 2-3); `CalendarPoller::run_loop`/`poll_once` iterate multiple connections (Decisions 4-5); `CALENDAR_URL_KEY` kept as a constant (needed for the migration read) alongside new `CALENDAR_CONNECTIONS_KEY`.

#### [MODIFY] `crates/commands/src/lib.rs`
New `Command::AddCalendar { label: String, url: String }`; dispatches to a new narrow port (mirrors how `Command::Connect` dispatches through `connectors: HashMap<IntegrationSource, Arc<dyn IntegrationConnector>>` — `AddCalendar` needs its own single-entry map or a direct `Option<Arc<dyn CalendarConnectionAdder>>` field, since it's not a source-generic operation the way `Connect`/`ApplySelection` are). `IntegrationSource::Calendar` gets registered into the existing `selection_appliers`/`pickers` maps in `crates/app/src/main.rs` (Decision 3) — no new generic plumbing needed there, `CalendarAdapter` just becomes one more entry.

#### [MODIFY] `crates/ui/src/state.rs`, `crates/ui/src/keyboard.rs`, `crates/ui/src/render.rs`, `crates/ui/src/lib.rs`
`CalendarSetupState` gains `label_input: String`/`editing_label: bool` alongside the existing `token_input`; new `KeyOutcome::SubmitCalendarConnection { label, url }`; `Ctrl+K` global shortcut opening the picker overlay (reusing `render_slack_picker_overlay`'s sibling, `render_github_picker_overlay`, as the template — Calendar's picker is single-list like GitHub's, not two-section like Slack's); help overlay gains the `Ctrl+K` entry.

#### [MODIFY] `crates/app/src/main.rs`
Wire `CalendarAdapter` into `pickers`/`selection_appliers`; wire the new `AddCalendar` port into `WorkspaceCommandHandler::new`.

---

## Verification Plan

- Unit tests: the old-key migration (a saved `CALENDAR_ICAL_URL` with no `CALENDAR_CONNECTIONS` becomes a real single-entry connection list on `initialize()`); adding a second calendar doesn't drop the first; `Picker::list_items()` reflects exactly the currently-connected set; `SelectionApplier::apply()` with a subset of ids actually removes the excluded ones (and persists the removal, not just an in-memory change); a poll cycle where one of two connections fails still reports overall `Success` and still delivers the other connection's reminders; occurrence titles carry the right label; `seen_occurrences` scoped per-connection doesn't cross-suppress two calendars that happen to share an event UID.
- Manual verification on this (Windows) machine: connect two real calendars via `Ctrl+L` twice, confirm both sets of reminders appear labeled correctly in the Notification/Calendar panel; open `Ctrl+K`, uncheck one, confirm only the other's reminders continue to arrive on the next poll cycle.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` all green.

---

## Implementation Notes (what actually happened)

Decisions 1, 4, and 5 shipped exactly as designed. Decisions 2 and 3's *outcomes* (label-then-URL entry; a checkbox picker for removal) shipped as designed too, but the **plumbing underneath both changed** once a real crate-dependency constraint was checked, in a way that ended up needing *less* new surface than the original proposal, not more.

**Found while implementing, not anticipated in the design**: `Picker` is defined in `crates/integration`, but `SelectionApplier` is defined in `crates/commands` — and `crates/commands` already depends on `crates/integration` (for `IntegrationConnector`/`Picker`/adapter types), so `crates/integration` implementing a `crates/commands` trait directly would be a dependency cycle. This is exactly why `GitHubAdapter` doesn't `impl SelectionApplier` itself — `ConfigFileGitHubSelectionApplier`, a separate bridge type living in `crates/app/src/main.rs` (which depends on both crates), does that instead, forwarding to a plain inherent method (`GitHubAdapter::update_selection`). Calendar needed the identical bridge shape (`CalendarSelectionApplierBridge`, forwarding to a new `CalendarAdapter::keep_only`) — but *not* a bridge for adding calendars, since `IntegrationConnector` (unlike `SelectionApplier`) lives in `crates/integration` itself, so `CalendarAdapter` can `impl IntegrationConnector` directly, exactly like it already did pre-`step24.md`.

**This made the originally-proposed `Command::AddCalendar` unnecessary.** Once "adding a calendar" could stay on the existing `IntegrationConnector::connect(token: String)` method, reusing the already-fully-wired `Command::Connect`/`KeyOutcome::SubmitToken`/`connectors: HashMap<IntegrationSource, ...>` plumbing was strictly simpler than introducing a new `Command` variant, a new narrow trait, and a new `WorkspaceCommandHandler` field just to carry two strings instead of one. `connect()`'s `token: String` parameter carries both fields as `"{label}\n{url}"` — safe because the label field's own capture logic never lets a literal newline into `label_input` (`Enter` always ends the field instead of inserting one). **Net result: `crates/commands` needed zero code changes for this entire phase** — every dispatch path (`Command::Connect`, `Command::ApplySelection`) was already fully generic across `IntegrationSource` from `step11.md`'s earlier generalization work.

**One real UX judgment call made during implementation, not pre-specified**: `Ctrl+K`'s picker rows start **checked**, not unchecked. GitHub's repo picker defaults every row unchecked (nothing is selected until the user actively adds it — a "discover and choose" flow). Calendar's picker shows only *already-connected* calendars, where the natural default action is "keep everything as-is" — defaulting to unchecked would have made an accidental `Enter` with no `Space` presses first look like "remove everything," the opposite of a safe default.

Final state: 68 tests in `crates/integration` (up from 66) — 8 new/rewritten calendar tests covering the legacy-key migration, adding a second calendar without dropping the first, `keep_only` actually persisting the removal (not just mutating in-memory state), and the partial-failure-still-reports-success poll behavior. 120 tests in `crates/ui` (up from 112) — the two-field setup capture (label-advances-not-submits, empty-label-doesn't-advance, combined submission), and the new `Ctrl+K` picker (open, space-toggle, submit-only-checked). `crates/app/src/main.rs` gained one new bridge type and two map registrations (`pickers`, `selection_appliers`); `crates/commands` untouched. Full `cargo fmt`/`check`/`clippy -D warnings`/`test --workspace` green with no regressions.
