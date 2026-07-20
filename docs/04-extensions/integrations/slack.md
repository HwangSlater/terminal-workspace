# Slack Integration

Implements `IntegrationAdapter` (`docs/04-extensions/integration-contract.md`) via Slack's Web API. See `step6.md` for the design decisions behind the choices below (polling over Socket Mode, Bot Token over full OAuth, honest-empty over fake demo data, a watch-list over whole-workspace presence).

## Authentication

- A **Bot Token** (`xoxb-...`), obtained by creating a Slack App in the target workspace (Slack's "Create New App" → "OAuth & Permissions" → install to workspace → copy the Bot User OAuth Token). This is a one-time, per-workspace setup step done by the user, not something this app automates — there is no OAuth authorization-code flow.
- Supplied via the `SLACK_BOT_TOKEN` environment variable, resolved through `SecretProviderChain::default_chain()` (ADR-0006) — `EnvProvider` picks it up with no additional code.
- No token found → `initialize()` returns `Ok(())` with the adapter reporting `ConnectionStatus::Disconnected`; see `integration-contract.md` §2.3.

## Permissions (OAuth Scopes)

The Slack App must be installed with these Bot Token scopes:
- `channels:history` — read messages from public channels the bot has been added to.
- `channels:read` — resolve channel IDs.
- `groups:history` / `groups:read` — the private-channel equivalents of the two scopes above (`step29.md`) — required for private channels to appear in the `Ctrl+P` picker and for `conversations.history` to work on them once selected. Without these, a private channel the bot was invited to still won't show up even after the `step29.md` `types` fix, since Slack's API itself will reject the request.
- `users:read` — resolve display names and presence for `watched_user_ids`.
- `chat:write` — required for `Command::SendSlackMessage` (`chat.postMessage`).

## Events

Two domain events flow out of this adapter into the Event Bus (`crates/events`), consumed by the existing `Projector` (`crates/commands`) with no changes needed there:
- `Event::SlackMessageReceived(NotificationItem)`
- `Event::SlackPresenceChanged(MemberPresence)`

## Receiving

Polling loop (`tokio::time::interval`, period = `[integrations.slack].sync_interval_secs`, default 30s). `/sync` (`step46.md`) forces one cycle immediately without waiting for the timer — the same `poll_once` this loop calls, invoked once out-of-band and serialized against the loop's own tick via a shared lock so the two can't race over the per-channel cursor.

1. **Messages**: `conversations.history` for each channel ID in `[integrations.slack].channel_ids`. New messages (by `ts`, tracked per-channel since the last successful poll — in-memory only, reset on every app restart) map to `NotificationItem`: `source = IntegrationSource::Slack`, `title` = sender display name, `body` = message text, `timestamp_ms` from Slack's `ts`, `priority = PriorityLevel::Medium`, `action_link = None` (no deep-link scheme defined yet). `is_read` is `true` only for the very first poll of a channel since this process started (`step33.md`/`step39.md`) — with an empty cursor, every message already in a channel's recent history looks "new," and without this it would fire a desktop notification on every single launch. That first poll still publishes normally (so the Notification panel is accurate immediately, `step39.md` — `step33.md`'s original fix skipped publishing entirely here, which meant a message that happened to already exist at the moment of that first poll was silently lost forever, not just delayed, since the cursor still advances past it either way) and still advances the per-channel cursor; only `DesktopNotifier` treats `is_read: true` differently, by not toasting for it.
2. **Presence**: `users.getPresence?user=<id>` for each ID in `[integrations.slack].watched_user_ids` (not the whole workspace — see `integration-contract.md`, this avoids one API call per teammate per cycle on a large workspace). Maps to `MemberPresence`: `status` = `Active` if Slack reports `"active"`, else `Away`; `Offline`/`Meeting`/`Lunch` are not derivable from this API and are left for a future enhancement (e.g. inferring from custom status emoji).

## Sending

`Command::SendSlackMessage { channel_id, text }` → `chat.postMessage`. Replaces the Phase 3 placeholder (`WorkspaceError::Integration("Slack integration not yet implemented")`) now that a real adapter exists to call.

## Reconnect

Not applicable in the polling model — there is no persistent connection to reconnect. See `integration-contract.md` §2.1 for the consecutive-failure counter that plays the equivalent role.

## Rate Limits

On a Slack `429`, read the `Retry-After` header, pause outbound calls for that duration, and skip the current poll cycle (`integration-contract.md` §2.2) — this does not count against the consecutive-failure threshold.

## Error Handling

- Non-2xx / network error / malformed JSON on a poll: log at `warn`, skip this cycle's update for that resource, continue the loop. Counts toward the consecutive-failure threshold (`integration-contract.md` §2.1).
- `chat.postMessage` failure (from `Command::SendSlackMessage`): surfaced synchronously as `Err(WorkspaceError::Integration(..))` to the command caller — sending is a direct user action, not a background sync, so silent skipping would be the wrong failure mode here.

## Picking channels/users (`step8.md`, `Ctrl+P`)

`channel_ids`/`watched_user_ids` no longer need hand-editing `config.toml` — the `Ctrl+P` overlay fetches live lists via `conversations.list` (`types=public_channel,private_channel` — see `step29.md`, `types` was `public_channel` only until a real bug report showed a just-invited private channel never appearing — filtered to `is_member: true` — only channels the bot has already been invited to, since `conversations.history` fails otherwise) and `users.list` (filtered to exclude bots and deleted accounts), both cursor-paginated. `channels:read`/`users:read` above cover public channels; a private channel additionally needs `groups:read`/`groups:history` (`step29.md`) or it still won't appear even with the corrected `types` param. Selecting rows and pressing `Enter` dispatches `Command::ApplySlackSelection`, which overwrites `config.toml`'s `channel_ids`/`watched_user_ids` (see Configuration below) and restarts the poll loop with them immediately — no restart required. Manually editing `config.toml` still works too; the picker is a convenience, not the only way in. **`step37.md`**: on a workspace with a lot of channels/people, scrolling to find one got impractical — `/` now starts a live, case-insensitive filter against the fetched labels (client-side, no extra API call), narrowing the visible rows as you type; `Enter` stops typing and returns to `j`/`k`/arrows/`Space` browsing the now-narrowed list. The filter only changes what's *visible* — an already-checked row stays checked and gets submitted on save even if a later search hides it from view. **`step41.md`**: `/slack-watch #channel [#channel2 ...]` dispatches the same `Command::ApplySlackSelection` from the command line, replacing the full channel list in one shot (channel names resolve against whatever `Ctrl+P` last fetched, same lookup `/send` already uses) — `watched_user_ids` is left untouched, carried over from the picker's last-known selection. No command-line equivalent adds/removes a single presence watch target yet; use `Ctrl+P` for that. **`step45.md`**: `Tab` now autocompletes every `#channel` argument of `/slack-watch` (not just `/send`'s single one), matched the same case-insensitive-prefix way.

The picker's second section, "사용자" (users), is unrelated to channels/DMs — selecting a person there adds them to `watched_user_ids`, a presence watch-list polled via `users.getPresence` (see Receiving below). It does not open, create, or list any kind of direct-message channel.

## Configuration

```toml
[integrations.slack]
enabled = true
sync_interval_secs = 30
channel_ids = ["C0123456789"]
watched_user_ids = ["U0123456789", "U0987654321"]
```

Token is **not** in this file — see Authentication above. `AppConfig::save_to` (used by the `Ctrl+P` picker and `/slack-watch`, `step41.md`) round-trips through `serde`, so hand-added comments/formatting elsewhere in this file are lost if either writes it — an accepted, documented limitation (`step8.md`), not a silent one. **`step42.md`**: it used to also read from a config snapshot cached once at process startup rather than the file's current contents, so any *other* field that changed on disk after startup (a newer default, a hand edit, another selection saved in between) got silently reverted on every save — a real bug a user hit directly (`step33.md`'s Verification section). Fixed by reading fresh from disk immediately before every save instead of relying on a cached copy.

## Testing

- Pure mapping functions (Slack JSON → `NotificationItem`/`MemberPresence`) unit-tested against fixture JSON, no network required.
- Rate-limit handling: mock a `429` + `Retry-After`, assert the cycle is skipped, not counted as a failure.
- No-token behavior: `initialize()` with an empty `SecretProviderChain` asserts `ConnectionStatus::Disconnected`, not an error and not synthetic data.
- Picker pagination/filtering (`conversations.list`/`users.list` cursor handling, `is_member`/`is_bot`/`deleted` filtering) unit-tested against fixture JSON, same pattern as the mapping functions above -- server-side filtering, not to be confused with `step37.md`'s client-side `/` label search, which is a pure `crates/ui` concern tested there (`SlackPickerState::visible_indices`), not here.
- No live-network integration test exists (no test Slack workspace / CI secret) — manual verification with a real `SLACK_BOT_TOKEN` is the acceptance check for this phase.
- `step39.md`: `poll_once` itself stays untested for the same live-network reason as above; the desktop-toast half of the fix (an `is_read: true` item produces no notification) is unit tested directly in `crates/notifications`. Manual acceptance check: send a real message immediately after a fresh restart, confirm it appears in the Notification panel without waiting for a second poll, confirm no desktop toast fires for it.
