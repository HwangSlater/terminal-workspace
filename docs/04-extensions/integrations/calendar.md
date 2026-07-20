# Calendar Integration

Implements `IntegrationAdapter` (`docs/04-extensions/integration-contract.md`) via one or more Google Calendars' **secret iCal feed URLs**, not OAuth. See `step12.md` for the original single-calendar design decisions (secret URL over OAuth device flow, a real `RRULE`-expansion dependency over hand-rolled parsing), `step24.md` for multi-calendar support, `step25.md` for the lookahead command, rename, and month grid view, `step26.md`/`step27.md` for the grid view's later polish (bigger popup, configurable dock widths, simplified navigation, a bigger centered grid), `step32.md` for the right-dock reminder panel's width becoming content-sized instead of fixed, with wrapping replaced by truncation, `step33.md` for a real bug fix — every occurrence already inside the lookahead window was firing a desktop notification on every single app launch — and `step39.md`, which corrected `step33.md`'s fix after it turned out to also leave the reminder panel empty for up to a full poll interval after every restart.

## Authentication

- Google Calendar → Settings → the specific calendar → "Integrate calendar" → **Secret address in iCal format**. This is a private, per-calendar HTTPS URL (`https://calendar.google.com/calendar/ical/<id>/private-<secret>/basic.ics`) that returns that calendar's events as a standard RFC 5545 feed, read-only. No app registration, no OAuth consent screen, no expiry.
- Supplied through the `Ctrl+L` setup overlay — a display label first (shown alongside that calendar's reminders, e.g. `[회사] Design Review`), then the secret URL. `Ctrl+L` **adds** a calendar rather than replacing the connection set; open it again to add another. Persisted via `SecretProviderChain` (same as Slack's Bot Token / GitHub's PAT — never `config.toml`), one JSON-serialized list under a single secret key, `CALENDAR_CONNECTIONS`.
- The URL itself is the bearer credential — leaking it grants read access to that calendar, nothing more. Same trust model as every other integration's token. The label is not a secret.
- No connections saved → `initialize()` returns `Ok(())` with the adapter reporting `ConnectionStatus::Disconnected`; see `integration-contract.md` §2.3.
- **Multiple calendars per connection (`step24.md`)**, up from the original one. There is still no "list my calendars" discovery API under this auth model, so `Ctrl+K`'s picker overlay (`Picker`/`SelectionApplier`) is a *local* read of already-connected calendars for removal — check which to keep, `Enter` removes the rest — not a remote discovery list the way Slack's/GitHub's pickers are. **`step41.md`**: `/calendar-remove <label>` does the same removal from the command line — resolves `<label>` against whatever `Ctrl+K` last fetched, then submits every *other* connected calendar's id (`Command::ApplySelection`'s "keep exactly these ids" shape). **`step43.md`**: `Ctrl+K` also gained the same `/`-to-search Slack's and GitHub's pickers already had (`step37.md`) — deliberately left out originally on the assumption a connected-calendar list stays short, added once the user actually had enough calendars connected for scrolling to matter. **`step45.md`**: `Tab` in the command bar now autocompletes `/calendar-remove`/`/calendar-rename`'s target-calendar argument against the same connected-calendar list, case-insensitively by prefix — matching a multi-word label (e.g. "개인 일정") from its first word splices the whole label in, but completing from partway through a later word isn't supported, the same limitation `split_calendar_label`'s longest-prefix parsing exists to route around at submit time.
- **Upgrade compatibility**: an install from before `step24.md` has a secret saved under the old singular key, `CALENDAR_ICAL_URL`. `initialize()` checks the new list key first and falls back to reading the old one (auto-labeled, since no label was ever collected for it) rather than dropping that connection on upgrade.
- **Renaming (`step25.md`)**: pressing `e` on a highlighted calendar in `Ctrl+K`'s picker opens a plain-text rename prompt, pre-filled with the current label. Only the label changes — a calendar's URL can't be edited in place, only removed (`Ctrl+K`) and re-added (`Ctrl+L`) with a new one, since revealing a masked secret field to edit it isn't meaningfully better UX than a fresh paste. **`step41.md`**: `/calendar-rename <기존 이름> <새 이름>` does the same rename from the command line. Labels are free text and can contain spaces, so the parser matches the *longest* connected calendar label that prefixes the typed text rather than splitting on the first space — `/calendar-rename 개인 일정 새 이름` correctly resolves the two-word label "개인 일정," not just "개인."

## Permissions

None to request — each secret URL is scoped to exactly one calendar, read-only, by Google. Connecting more than one is more of these, not a broader grant.

## Events

One domain event flows out of this adapter into the Event Bus (`crates/events`), consumed by the existing `Projector` (`crates/commands`) with no changes needed there — `Event::CalendarReminderTriggered(NotificationItem)` already existed in the frozen `Event` enum before this phase; this adapter is simply its first producer:
- `Event::CalendarReminderTriggered(NotificationItem)`
- `Event::IntegrationStatusChanged { source: IntegrationSource::Calendar, status }` — reuses the generic status event from Phase 9 (ADR-0016).

## Receiving

Polling loop (`tokio::time::interval`, period = `[integrations.calendar].sync_interval_secs`, default 900s — no point polling faster than Google's own feed cache refreshes) iterates **every** configured calendar each cycle, same shared interval for all (`step24.md` — nothing asks for a per-calendar interval). `/sync` (`step46.md`) forces one cycle immediately without waiting for the timer, the same way it does for Slack/GitHub:

1. For each connection: `GET` its secret iCal URL, parse the returned `VCALENDAR` (`ical` crate).
2. For each `VEVENT`, expand its occurrences via `RRuleSet` (`rrule` crate): the `DTSTART` (and `RRULE`/`EXDATE` if present) are reassembled into raw iCal property lines and parsed as an `RRuleSet`, which handles both the date/time format and any `TZID` timezone qualifier correctly — this is exactly the part not worth hand-rolling (see `step12.md`'s Context for why). A `VEVENT` with no `RRULE` gets its `DTSTART` injected as an explicit `RDATE` line first, since `RRuleSet`'s iterator only ever draws from `RRULE`/`RDATE` entries and yields nothing for a bare `DTSTART` on its own (a real bug caught during implementation — see `step12.md`'s Implementation Notes).
3. Only occurrences starting within `[now, now + lookahead_hours)` become a notification (`lookahead_hours`, default 24) — a reminder feature, not a full calendar dump.
4. Dedup via an in-memory `(calendar connection id, event UID, occurrence start epoch millis)` seen-set — the connection id is included (`step24.md`) so two different calendars can never cross-suppress each other even in the extremely unlikely case they share an event UID. In-memory only, reset on every app restart. The very first poll after every launch (or a connection/selection change) still populates this set with everything it finds and **still publishes normally** (`step39.md` — `step33.md`'s original fix skipped the publish entirely on this first poll, which meant the reminder panel stayed empty for up to a full `sync_interval_secs` after every restart, since nothing else ever populates it); the occurrence's `is_read` is set to `true` for this first-poll case instead, so only the desktop toast is suppressed, not the panel entry.
5. Maps to `NotificationItem`: `source = IntegrationSource::Calendar`, `title` = `"[{label}] {SUMMARY}"` (the connection's label prefixed, `step24.md` — how multiple calendars stay distinguishable once merged into one panel), `body` = the event's `LOCATION` if present, `timestamp_ms` = the occurrence's start time, `priority = PriorityLevel::Medium`, `action_link` = the event's `URL` property if the feed provides one (`is_read` set separately per bullet 4 above, not a fixed `false`).
6. **One bad calendar doesn't mask the others working** (`step24.md`): the poll cycle as a whole only reports `Failure` if *every* configured calendar failed that cycle. Each failing connection still logs its own reason independently (`tracing::warn!`, visible via `Ctrl+c`'s log viewer).

## Month Grid View (`step25.md`)

`Ctrl+M` opens a real month calendar grid — the right dock's "upcoming reminders" list stays a flat, `lookahead_hours`-bounded list; the grid is a separate, on-demand view with its own fetch, entirely independent of the reminder poll loop:

- `CalendarManager::events_in_range(after, before)` re-fetches every connected calendar fresh (via the same `fetch_calendar_feed` helper the poll loop uses) and expands occurrences for `[after, before)` — a whole month, not `lookahead_hours`. No dedup/seen-state interaction (that's specific to "fire a reminder once"); no publish to the Event Bus (this is a read, not a trigger).
- Days with at least one event get a marker (`●`, yellow) in the grid; the highlighted day's events are listed by time and title underneath, each with its own `●` bullet. Weekends read in the usual calendar-app convention (Sunday red, Saturday blue), and today's real date is always highlighted (bold cyan) independent of whatever day is currently selected (`step28.md`).
- Left/Right arrow keys move the day cursor within the displayed month, clamped to its real day count — no wraparound into an adjacent month (that would need a mid-navigation re-fetch, deliberately out of scope). `h`/`j`/`k`/`l` deliberately do nothing here, unlike every other overlay in this app — requested directly, since the letters read as ordinary text in a date-navigation context rather than clearly-a-shortcut the way they do in a plain list picker (`step26.md`). Up/Down were dropped too (`step27.md`) — the week-jump behavior itself, not just its letter-key alternative. `[`/`]` explicitly change the displayed month and trigger a fresh fetch. The highlighted day's event list below the grid shows each event's local time, not just its title. The day-number grid itself renders wider, spaced-out cells, horizontally centered within the popup rather than flush against its left edge (`step27.md`), with a centered, bold month/year title (`step28.md`).
- The fetch range uses UTC month boundaries rather than local-midnight-with-DST-handling — a deliberate simplification, since the range only bounds *which* occurrences get fetched; which day cell each one's marker lands on is computed correctly in local time separately.

## Lookahead Range Command (`step25.md`)

`/calendar-range <hours>` changes `lookahead_hours` at runtime via `CalendarManager::set_lookahead_hours`, which updates the config and restarts polling (the running poll loop snapshotted its config once at `start()` time, so a live config change needs a restart to take effect — same pattern `keep_only` already established).

## Sending

None. Read-only — same reasoning as GitHub (`step10.md` Decision 1): nothing in the product requirements asks for calendar writes.

## Reconnect

Not applicable in the polling model — there is no persistent connection to reconnect. See `integration-contract.md` §2.1 for the consecutive-failure counter that plays the equivalent role.

## Rate Limits

Google doesn't publish an explicit rate limit for this feed endpoint the way Slack/GitHub's APIs do — it's a cached, mostly-static resource. A `429` is still handled defensively (pause and skip the cycle, per `integration-contract.md` §2.2) in case one is ever returned.

## Error Handling

- Non-2xx / network error / malformed ICS on a poll: log at `warn`, skip this cycle, continue the loop. Counts toward the consecutive-failure threshold (`integration-contract.md` §2.1).
- A single malformed `VEVENT` (unexpected property shape, an `RRULE` `RRuleSet` can't parse) is skipped individually rather than failing the whole poll cycle — the same "degrade, don't crash the batch" instinct as Slack's missing-display-name fallback.

## Configuration

```toml
[integrations.calendar]
enabled = true
sync_interval_secs = 900
lookahead_hours = 24
```

The secret iCal URL is **not** in this file — see Authentication above.

## Testing

- Pure mapping/expansion functions (ICS text → occurrences → `NotificationItem`) unit-tested against fixture `.ics` text, including a genuinely recurring `VEVENT` (a daily-standup shape) to prove `RRULE` expansion actually works, not just parses.
- The non-recurring-event RDATE-injection fix specifically tested: a bare `DTSTART` with no `RRULE` yields exactly one occurrence inside a wide window, and zero outside it.
- No-credential behavior: `initialize()` with an empty `SecretProviderChain` asserts `ConnectionStatus::Disconnected`, not an error and not synthetic data.
- No live-network integration test exists (no test Google Calendar / CI secret) — manual verification with a real secret iCal URL is the acceptance check for this phase.
- `step24.md`: the legacy-single-URL migration path, adding a second calendar without dropping the first, `Picker`/`SelectionApplier` reflecting exactly the connected set, and a partial-failure poll cycle (one bad calendar, one good) still delivering the good one's reminders and still reporting overall success.
- `step25.md`: `set_lookahead_hours` actually updates the config and keeps polling (not left disconnected); `rename` updates and persists the label without touching the URL or polling; a real, unknown-id rename is an error; the `days_in_month`/`shift_month` pure helpers behind the grid's navigation (normal months, February in leap vs. non-leap years, December→January year rollover in both directions).
- `step26.md`/`step27.md`: a custom `[layout]` dock width actually reaches the rendered buffer's column boundaries (exact cell-position checks, not text search); Up/Down no longer move the grid's day cursor at all (`step27.md`, not just their `h`/`j`/`k`/`l` alternative from `step26.md`); the weekday header renders horizontally centered in the popup rather than flush against its left edge.
- `step32.md`: a light day's reminders render the right-dock panel narrower than the configured `right_dock_width` ceiling; a configured ceiling that's smaller than an outlier title's real width still reaches the buffer exactly; a title too long even at the ceiling is truncated with an ellipsis and never wrapped, under any circumstance.
- `step33.md`/`step39.md`: no direct unit test (`poll_one`/`poll_once` are live-network code, same "not directly unit tested" boundary as the rest of this section) — the desktop-toast suppression half of `step39.md`'s correction is unit tested in `crates/notifications` instead. Manual acceptance check: launch with an existing calendar connection and upcoming occurrences already inside the lookahead window, confirm they appear in the reminder panel immediately (not after waiting a full poll interval) and confirm no desktop toast fires for them on that first poll, then confirm a later poll's genuinely new occurrence both appears and notifies normally.
