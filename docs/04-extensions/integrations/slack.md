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
- `users:read` — resolve display names and presence for `watched_user_ids`.
- `chat:write` — required for `Command::SendSlackMessage` (`chat.postMessage`).

## Events

Two domain events flow out of this adapter into the Event Bus (`crates/events`), consumed by the existing `Projector` (`crates/commands`) with no changes needed there:
- `Event::SlackMessageReceived(NotificationItem)`
- `Event::SlackPresenceChanged(MemberPresence)`

## Receiving

Polling loop (`tokio::time::interval`, period = `[integrations.slack].sync_interval_secs`, default 30s):
1. **Messages**: `conversations.history` for each channel ID in `[integrations.slack].channel_ids`. New messages (by `ts`, tracked per-channel since the last successful poll) map to `NotificationItem`: `source = IntegrationSource::Slack`, `title` = sender display name, `body` = message text, `timestamp_ms` from Slack's `ts`, `priority = PriorityLevel::Medium`, `is_read = false`, `action_link = None` (no deep-link scheme defined yet).
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

## Configuration

```toml
[integrations.slack]
enabled = true
sync_interval_secs = 30
channel_ids = ["C0123456789"]
watched_user_ids = ["U0123456789", "U0987654321"]
```

Token is **not** in this file — see Authentication above.

## Testing

- Pure mapping functions (Slack JSON → `NotificationItem`/`MemberPresence`) unit-tested against fixture JSON, no network required.
- Rate-limit handling: mock a `429` + `Retry-After`, assert the cycle is skipped, not counted as a failure.
- No-token behavior: `initialize()` with an empty `SecretProviderChain` asserts `ConnectionStatus::Disconnected`, not an error and not synthetic data.
- No live-network integration test exists (no test Slack workspace / CI secret) — manual verification with a real `SLACK_BOT_TOKEN` is the acceptance check for this phase.
