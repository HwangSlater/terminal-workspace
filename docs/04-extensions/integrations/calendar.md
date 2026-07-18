# Calendar Integration

Implements `IntegrationAdapter` (`docs/04-extensions/integration-contract.md`) via a Google Calendar's **secret iCal feed URL**, not OAuth. See `step12.md` for the design decisions behind the choices below (secret URL over OAuth device flow, a real `RRULE`-expansion dependency over hand-rolled parsing).

## Authentication

- Google Calendar → Settings → the specific calendar → "Integrate calendar" → **Secret address in iCal format**. This is a private, per-calendar HTTPS URL (`https://calendar.google.com/calendar/ical/<id>/private-<secret>/basic.ics`) that returns that calendar's events as a standard RFC 5545 feed, read-only. No app registration, no OAuth consent screen, no expiry.
- Supplied through the `Ctrl+L` setup overlay (persisted via `SecretProviderChain`, same as Slack's Bot Token / GitHub's PAT — never `config.toml`) or the `CALENDAR_ICAL_URL` environment variable.
- The URL itself is the bearer credential — leaking it grants read access to that calendar, nothing more. Same trust model as every other integration's token.
- No token found → `initialize()` returns `Ok(())` with the adapter reporting `ConnectionStatus::Disconnected`; see `integration-contract.md` §2.3.
- **Single calendar per connection.** There is no "list my calendars" API under this auth model, so there's nothing for a picker to discover — unlike Slack/GitHub, this integration has no `Ctrl+R`-style picker overlay at all.

## Permissions

None to request — the secret URL itself is scoped to exactly one calendar, read-only, by Google.

## Events

One domain event flows out of this adapter into the Event Bus (`crates/events`), consumed by the existing `Projector` (`crates/commands`) with no changes needed there — `Event::CalendarReminderTriggered(NotificationItem)` already existed in the frozen `Event` enum before this phase; this adapter is simply its first producer:
- `Event::CalendarReminderTriggered(NotificationItem)`
- `Event::IntegrationStatusChanged { source: IntegrationSource::Calendar, status }` — reuses the generic status event from Phase 9 (ADR-0016).

## Receiving

Polling loop (`tokio::time::interval`, period = `[integrations.calendar].sync_interval_secs`, default 900s — no point polling faster than Google's own feed cache refreshes):

1. `GET` the secret iCal URL, parse the returned `VCALENDAR` (`ical` crate).
2. For each `VEVENT`, expand its occurrences via `RRuleSet` (`rrule` crate): the `DTSTART` (and `RRULE`/`EXDATE` if present) are reassembled into raw iCal property lines and parsed as an `RRuleSet`, which handles both the date/time format and any `TZID` timezone qualifier correctly — this is exactly the part not worth hand-rolling (see `step12.md`'s Context for why). A `VEVENT` with no `RRULE` gets its `DTSTART` injected as an explicit `RDATE` line first, since `RRuleSet`'s iterator only ever draws from `RRULE`/`RDATE` entries and yields nothing for a bare `DTSTART` on its own (a real bug caught during implementation — see `step12.md`'s Implementation Notes).
3. Only occurrences starting within `[now, now + lookahead_hours)` become a notification (`lookahead_hours`, default 24) — a reminder feature, not a full calendar dump.
4. Dedup via an in-memory `(event UID, occurrence start epoch millis)` seen-set, same pattern as GitHub's `(repo, pr_number)` set.
5. Maps to `NotificationItem`: `source = IntegrationSource::Calendar`, `title` = the event's `SUMMARY`, `body` = its `LOCATION` if present, `timestamp_ms` = the occurrence's start time, `priority = PriorityLevel::Medium`, `is_read = false`, `action_link` = the event's `URL` property if the feed provides one.

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
