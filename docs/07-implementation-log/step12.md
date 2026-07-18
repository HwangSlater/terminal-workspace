# Implementation Plan - Phase 12: Calendar Integration (v0.4)

This is a **design document for review — nothing described below has been implemented yet**, per the same process used for Phases 6-11.

## Context

`docs/01-product/roadmap.md` lists `v0.4 Calendar` next. Some groundwork already exists and doesn't need to be re-invented: `Event::CalendarReminderTriggered(NotificationItem)` already exists in the frozen `Event` enum (added speculatively, like `GitHubPRCreated` was — this phase is simply its first producer, no ADR needed); `IntegrationSource::Calendar` already exists; `docs/05-operations/configuration.md` §1 already sketches `[integrations.calendar]`; `docs/04-extensions/integrations/calendar.md` is a bare outline, same starting state `github.md` was in before Phase 10.

**What's genuinely different about Calendar, and why this isn't just "GitHub again"**: Slack (Bot Token) and GitHub (PAT) both have a simple, static, paste-once bearer secret issued directly by the service. **Google Calendar has no equivalent for a personal calendar.** The two realistic options are:

1. **OAuth 2.0** (authorization-code or device-flow) — requires this project to register and ship its own OAuth Client ID with Google, and for device-flow specifically, an HTTP polling loop against Google's token endpoint plus refresh-token rotation (access tokens expire in ~1 hour). This is exactly the "real work with no payoff yet" `step6.md` Decision 2 declined for Slack, now with the added complexity of a token that expires and must be silently renewed rather than a fire-and-forget static string.
2. **A calendar's "Secret address in iCal format"** — Google Calendar Settings → a specific calendar → "Integrate calendar" exposes a private HTTPS URL (`https://calendar.google.com/calendar/ical/<id>/private-<secret>/basic.ics`) that returns that calendar's events as a standard iCalendar (RFC 5545) feed, read-only, no OAuth, no registered client, no expiry. The URL itself *is* the bearer credential — same trust model as Slack's Bot Token or GitHub's PAT (leak it, someone can read the calendar; nothing more).

Option 2 fits this project's established zero-new-infrastructure pattern exactly — it reuses `SecretProviderChain`/`IntegrationConnector` (`step11.md`) with zero new auth plumbing, at the cost of only supporting one calendar per connection (no "list my calendars" discovery API exists under this model, so there's nothing for a `Picker` to list — Calendar needs **no picker overlay at all**, simpler than GitHub) and Google's documented cache lag on this feed (their own docs say it can take "up to several hours" to reflect changes — not appropriate for something needing near-real-time sync, acceptable for a periodic reminder check).

**What's genuinely hard, and shouldn't be hand-rolled**: parsing the feed isn't the risky part (RFC 5545's line format is simple) — **expanding recurring events (`RRULE`) is**. `product-requirements.md`'s own flagship scenario is *"checks calendar for the standup link"* — a daily/weekly recurring event is the overwhelmingly common real case, not the exception. Getting `RRULE` expansion subtly wrong (off-by-one on `UNTIL`, wrong weekday math, missed `EXDATE` exceptions) produces silently-wrong reminders, which is worse than not having the feature. Unlike `step10.md`'s hand-rolled ISO 8601 parser (a simple, fully-specified format, genuinely easy to get right by hand), `RRULE` expansion is real, nontrivial, well-trodden ground that dependencies exist specifically to solve correctly.

---

## Decisions (confirmed)

### 1. Authentication: iCal secret-URL vs. OAuth (device flow)

**Confirmed**: iCal secret-URL. User pastes the secret feed URL (obtained from Google Calendar's own UI, no app registration) through a `Ctrl+L` setup overlay, mirroring Slack's `Ctrl+S`/GitHub's `Ctrl+G` exactly and reusing `IntegrationConnector` from `step11.md`'s registry as-is (the "token" is just the URL string). Read-only, single calendar per connection, subject to Google's own cache-refresh lag. Fits everything already built (`SecretProviderChain`, `IntegrationConnector`) with zero new auth infrastructure, matching the exact reasoning Slack's Decision 2 and GitHub's Decision 2 already used. Multi-calendar support and tighter sync latency (OAuth) are real capabilities left on the table, but nothing in the product requirements asks for them yet — revisitable later without an ADR (`IntegrationAdapter` isn't frozen).

### 2. iCal/RRULE parsing: a pure-Rust dependency vs. hand-rolled single-occurrence-only parsing

**Confirmed**: pull in dependencies for this — a pure-Rust iCalendar component parser plus a pure-Rust `RRULE` expansion crate (exact crates to be confirmed during implementation — must build without a C compiler, per ADR-0014's standing constraint, verified via a clean build the same way the Phase 6 `rustls`/`ring` regression was caught). Recurring events get expanded correctly rather than silently skipped. `step10.md`'s "hand-roll it, avoid a dependency" call was right for ISO 8601 specifically *because* it's simple and fully deterministic to implement correctly in a few lines; `RRULE` expansion doesn't have that property, and getting it wrong silently produces incorrect reminders rather than an obvious crash — the worse failure mode. This project already accepts real dependencies where the alternative is reimplementing genuinely hard logic (`reqwest`, `redb`, `keyring`, `aes-gcm`); this is that case.

### 3. Lookahead window for "upcoming" reminders

**Recommendation (not asking — low-stakes default)**: a configurable `[integrations.calendar] lookahead_hours` (default `24`) — only events starting within the next N hours become a notification, mirroring "what's coming up soon" rather than surfacing the entire feed history/future at once. Dedup via a `(event UID, occurrence start time)` seen-set, same pattern as GitHub's `(repo, pr_number)` set (`step10.md`).

---

## Proposed Changes (pending Decisions 1-2 above)

#### [MODIFY] `docs/04-extensions/integrations/calendar.md`
Replace the bare outline with a real spec: how to obtain the secret iCal URL, the feed format, lookahead window, `RRULE` expansion caveat (whichever dependency is chosen), `NotificationItem` field mapping.

#### [MODIFY] `crates/config/src/lib.rs`
`IntegrationsToggle` gains `calendar: CalendarSettings` (nested table, mirroring `GitHubSettings`): `enabled`, `sync_interval_secs` (default likely 300-900s — no point polling faster than Google's own cache refreshes), `lookahead_hours` (default 24). No `calendar_ids` list (Decision 1 — one calendar per connection; the aspirational multi-id sketch in `configuration.md` §1 was written before this constraint was known and gets corrected here). `#[serde(default)]` throughout, same Phase 6 lesson.

#### [NEW] `crates/integration/src/calendar.rs`
- `CalendarConfig { lookahead_hours: u64, sync_interval_secs: u64 }`.
- `CalendarAdapter` implementing `IntegrationAdapter` and `IntegrationConnector` (from `step11.md`'s registry — no new connector trait needed, the URL is just a `String` token). **No `Picker` impl** — nothing to list under this auth model (Decision 1's consequence).
- Polling loop reusing `crates/integration/src/polling.rs`'s shared failure-counter/rate-limit-adjacent machinery (`step10.md`'s hoist) — fetches the ICS feed, expands recurring events via the chosen crate, filters to the lookahead window, maps new (not-yet-seen) occurrences to `NotificationItem` (`source = IntegrationSource::Calendar`, `title` = event summary, `body` = location/description if present, `timestamp_ms` = occurrence start, `action_link` = the event's `URL` property if the feed provides one), publishes `Event::CalendarReminderTriggered` + `Event::IntegrationStatusChanged`.

#### [MODIFY] `crates/app/src/main.rs`
Construct `CalendarAdapter` alongside Slack/GitHub, register it in the `connectors` `HashMap` (`step11.md`) — **no new `WorkspaceCommandHandler`/`TuiRenderer` fields at all**, the registry absorbs it. This is the payoff `step11.md` was built for.

#### [MODIFY] `crates/ui/src/keyboard.rs`, `state.rs`, `render.rs`
`Ctrl+L` → `OverlayKind::CalendarSetup` (mirrors `Ctrl+G`'s `capture_github_setup_input`, reusing `KeyOutcome::SubmitToken(IntegrationSource::Calendar, url)` — no new `KeyOutcome` variant needed, `step11.md`'s generalization already covers this). `CalendarSetupState`/`CalendarSetupStatus` structurally identical to `GitHubSetupState`/`GitHubSetupStatus`. Header status line extends to a third `connection_status_label` call. **No picker overlay/keybinding** (Decision 1's consequence).

---

## Verification Plan

- Unit tests for the ICS → `NotificationItem` mapping against fixture `.ics` text, including at least one genuinely recurring `VEVENT` (weekly standup shape) to prove Decision 2 actually works, not just parses.
- Unit test proving the lookahead-window filter excludes both past occurrences and occurrences beyond the window.
- Unit test proving dedup: re-polling the same feed with an unchanged upcoming occurrence does not re-publish it.
- `CalendarAdapter::initialize` with no credential — asserts `Disconnected`, no synthetic data (`integration-contract.md` §2.3, same as every prior adapter).
- No live-network integration test (no test Google Calendar / CI secret) — manual verification: connect a real secret iCal URL with a recurring event and confirm a reminder appears within the lookahead window.

---

## Implementation Notes (what actually happened)

- **Dependencies confirmed pure-Rust, verified rather than assumed**: `ical = "0.11"` (parsing) + `rrule = "0.14"` (RRULE expansion, pulling in `chrono`+`chrono-tz`) were dry-run added and built cleanly with no C compiler in the dependency tree (`cargo tree` showed only pure-Rust crates — same verification discipline as the Phase 6 `rustls`/`ring` finding). Default features trimmed (`ical`'s unused `vcard`; `rrule`'s unused `exrule`/`by-easter`).
- **A real bug caught by reading the `rrule` crate's own source, not by guessing**: the original plan assumed feeding *every* `VEVENT` through `RRuleSet::from_str` uniformly (recurring or not) would work, since a bare `DTSTART` with no `RRULE` seemed like it should trivially yield itself as one occurrence. Reading `rrule` 0.14's `RRuleSetIter::into_iter` directly showed this is false: the iterator only ever draws from `self.rrule` and `self.rdate` — a `DTSTART` with neither yields **zero** occurrences. Uncaught, this would have meant **every non-recurring event** (the majority of real calendar entries — most meetings aren't recurring) silently never produced a reminder, while only recurring ones worked. Fixed by synthesizing an explicit `RDATE` line from `DTSTART`'s own value whenever no `RRULE` is present, forcing the single occurrence into the set the iterator actually draws from. Caught before ever running a test, by reading `rruleset_iter.rs`'s `IntoIterator` impl line by line.
- **Verification required installing a local MinGW toolchain, done with explicit confirmation first**: this dev machine's default Rust host is `x86_64-pc-windows-gnu` (this project's Experimental tier, not the Tier 1 MSVC target CI gates on) and had no `dlltool` on `PATH` — `chrono`'s `iana-time-zone` dependency needs it to link against `windows-core` FFI bindings for local-timezone detection. Confirmed with the user before running `winget install BrechtSanders.WinLibs.POSIX.UCRT` (the same lightweight, installer-free distribution the README already documents for this exact gap) rather than assuming it was fine to install something, given this project's specific prior history with a stuck toolchain install (ADR-0014's Context). Once installed, the RRULE fix above was empirically verified via real `cargo test` runs, not left as a source-reading-only inference.
- **No `Picker`/`SelectionApplier`/`ConfigFileCalendarSelectionApplier` exists for Calendar**, and this is not an oversight — `step12.md` Decision 1's consequence (no discovery API under the secret-URL auth model) meant `crates/app/src/main.rs`'s Calendar wiring is the simplest of the three integrations: construct, `initialize`, register one `IntegrationConnector` into the existing `step11.md` registry, done. No new `WorkspaceCommandHandler`/`Command` surface at all — exactly the payoff `step11.md` was built for.
- **`TuiRenderer` gained a third named `initial_calendar_status` constructor parameter** (not folded into a registry) — `WorkspaceState`'s per-integration connection-status/setup-overlay fields stay named per the established precedent (only `Command`, `WorkspaceCommandHandler`'s connector/applier fields, and `TuiRenderer`'s single-list picker port were registry-generalized in `step11.md`; overlay UI state for each integration is small and independently-shaped enough that it wasn't part of that generalization).
- **Verification reality**: `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace` all ran and passed with the newly-available toolchain. Test counts: `integration` 66 (up from 54; 12 new `calendar` tests including the recurring/non-recurring window tests that proved the RDATE fix), `config` 15 (up from 13), `ui` 69 (up from 62). No live Google Calendar was available to test the actual HTTP fetch against a real secret iCal URL — that remains a manual verification step, same caveat as every prior adapter's Phase.
