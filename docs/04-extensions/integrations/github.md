# GitHub Integration

Implements `IntegrationAdapter` (`docs/04-extensions/integration-contract.md`) via GitHub's REST API. See `step10.md` for the design decisions behind the choices below (open-PR diff over webhook/update tracking, Personal Access Token over OAuth App, full connect-UI + picker treatment built in the same phase as the polling adapter, unlike Slack which spread that across Phases 6-9).

## Authentication

- A **Personal Access Token** (classic, `ghp_...`), created at GitHub → Settings → Developer settings → Personal access tokens, with the `repo` scope. This is a one-time, per-account setup step done by the user — there is no OAuth App / authorization-code flow.
- Supplied either via the `GITHUB_TOKEN` environment variable (resolved through `SecretProviderChain::default_chain()`, ADR-0006) or entered in-app through the `Ctrl+G` setup overlay, which persists it the same way Slack's `Ctrl+S` does (OS keyring, falling back to an encrypted local file — never `config.toml`).
- No token found → `initialize()` returns `Ok(())` with the adapter reporting `ConnectionStatus::Disconnected`; see `integration-contract.md` §2.3.

## Permissions (PAT Scopes)

- `repo` — read access to pull requests (needed for private repositories; public repos work with an authenticated request even without this scope, but `repo` covers both without needing to special-case visibility).

## Events

One domain event flows out of this adapter into the Event Bus (`crates/events`), consumed by the existing `Projector` (`crates/commands`) with no changes needed there — `Event::GitHubPRCreated(NotificationItem)` already existed in the frozen `Event` enum before this phase; this adapter is simply its first producer:
- `Event::GitHubPRCreated(NotificationItem)`
- `Event::IntegrationStatusChanged { source: IntegrationSource::GitHub, status }` — reuses the generic status event added in Phase 9 (ADR-0016), no GitHub-specific event needed for connection status.

## Receiving

Polling loop (`tokio::time::interval`, period = `[integrations.github].sync_interval_secs`, default 60s):

- **Pull requests**: `GET /repos/{owner}/{repo}/pulls?state=open` for each repository in `[integrations.github].repositories`. A PR number not previously seen (tracked in an in-memory `(repo, pr_number)` set, per adapter instance) maps to `NotificationItem`: `source = IntegrationSource::GitHub`, `title` = `"{repo}#{number} {pr title}"`, `body` = `"by {author login}"`, `timestamp_ms` from the PR's `updated_at` (ISO 8601), `priority = PriorityLevel::Medium`, `is_read = false`, `action_link` = the PR's `html_url`.
- **First-connect behavior**: on the very first poll cycle after a connection, every currently-open PR is "new" (the seen-set starts empty) and is surfaced as a notification — a deliberate catch-up behavior, not a bug, mirroring Slack's own `conversations.history` returning recent messages on first connect rather than only future ones.
- **Not tracked**: PR updates or closes — only creation, matching the event name. Detecting those would need a new `Event` variant (frozen enum, needs an ADR) and was out of scope for this phase (`step10.md` Decision 3).

## Sending

None. This integration is read-only for this phase — no `Command::CommentOnPR`/`ApprovePR` equivalent to `SendSlackMessage` exists, since nothing in the current product requirements asks for GitHub writes yet (`step10.md` Decision 1).

## Reconnect

Not applicable in the polling model — there is no persistent connection to reconnect. See `integration-contract.md` §2.1 for the consecutive-failure counter that plays the equivalent role.

## Rate Limits

GitHub's authenticated REST quota (5,000 req/hr) is generous compared to Slack's per-method limits, but abuse-detection responses still occur. On a `429`, or a `403` accompanied by either a `Retry-After` header or `X-RateLimit-Remaining: 0`, pause outbound calls and skip the current poll cycle (`integration-contract.md` §2.2) — this does not count against the consecutive-failure threshold. A **plain `403` with neither header** (e.g. a bad or insufficient-scope token) is deliberately *not* treated as a rate limit — conflating the two would let an expired PAT retry forever without ever surfacing the real problem via the `Reconnecting`/`Failed` threshold.

## Error Handling

- Non-2xx / network error / malformed JSON on a poll: log at `warn`, skip this cycle's update for that repository, continue the loop. Counts toward the consecutive-failure threshold (`integration-contract.md` §2.1).
- Picker/setup calls (interactive, not background polling): surfaced synchronously as `Err(WorkspaceError::Integration(..))` to the caller, same reasoning as Slack's picker errors — the user is watching, a silent retry would look like a freeze.

## Picking repositories (`step10.md`, `Ctrl+R`)

`repositories` no longer needs hand-editing `config.toml` — the `Ctrl+R` overlay fetches the authenticated user's accessible repositories via `GET /user/repos` (page-paginated, stops once a page returns fewer than 100 results). Selecting rows and pressing `Enter` dispatches `Command::ApplySelection{source: IntegrationSource::GitHub, items}` (generalized in `step11.md` from `Command::ApplyGitHubSelection`, once Calendar-shaped single-list selections proved the pattern repeats), which overwrites `config.toml`'s `repositories` (see Configuration below) and restarts the poll loop with them immediately — no restart required. Manually editing `config.toml` still works too; the picker is a convenience, not the only way in.

## Configuration

```toml
[integrations.github]
enabled = true
sync_interval_secs = 60
repositories = ["rust-lang/rust", "google/terminal-workspace"]
```

Token is **not** in this file — see Authentication above. `AppConfig::save_to` (used by the `Ctrl+R` picker) round-trips through `serde`, so hand-added comments/formatting elsewhere in this file are lost if the picker writes it — same accepted, documented limitation as Slack's picker (`step8.md`).

## Testing

- Pure mapping function (GitHub JSON → `NotificationItem`) unit-tested against fixture JSON, no network required.
- Rate-limit handling: `403`+`Retry-After`, `403`+`X-RateLimit-Remaining: 0`, and plain `429` all recognized; a plain `403` with neither header is explicitly asserted to NOT be treated as rate-limited (the token-expiry footgun above).
- No-token behavior: `initialize()` with an empty `SecretProviderChain` asserts `ConnectionStatus::Disconnected`, not an error and not synthetic data.
- ISO 8601 timestamp parsing (hand-rolled, no `chrono`/`time` dependency — see `step10.md`) unit-tested against a known epoch value and the Unix epoch itself.
- No live-network integration test exists (no test GitHub account / CI secret) — manual verification with a real `GITHUB_TOKEN` is the acceptance check for this phase.
