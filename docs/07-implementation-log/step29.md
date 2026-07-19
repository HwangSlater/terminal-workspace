# Implementation Plan - Phase 29: Slack picker fixes and app-wide j/k hint removal

Real bug fixes plus a documentation-scope request, not new features — skips
the Decisions/AskUserQuestion cycle (nothing here has more than one
reasonable implementation), documented per `development.md`'s rule that a
change still needs a record even when it didn't need up-front confirmation.

## Context

Four issues reported from live use of `Ctrl+P` (Slack's channel/user
picker):

1. A channel the bot was just invited to didn't appear in the picker at
   all — suspected to be because it's a private channel.
2. Confusion about what the picker's second section, "사용자" (users),
   actually does.
3. With a long list, pressing the down arrow moves the cursor but the
   *visible* list doesn't scroll — the highlighted row goes off-screen
   with nothing to indicate it's still selected.
4. A request to remove "move with j/k" from the help text everywhere in
   the app, not just the Calendar grid (`step26.md`/`step27.md`).

## Fixes

### 1. Private channels never appeared (`crates/integration/src/slack.rs`)

Real bug, not a permissions misunderstanding on the user's part (though a
permissions gap exists too — see below). `fetch_channel_list`'s
`conversations.list` call hardcoded `("types", "public_channel")`. Slack's
API only returns the conversation types explicitly listed in `types` —
a private channel is invisible to this call no matter how the bot's
membership or scopes are set up. Changed to
`"public_channel,private_channel"`.

This alone isn't sufficient for private channels to actually appear or
work: the Bot Token also needs `groups:read` (to list them) and
`groups:history` (for `conversations.history` to succeed once one is
selected) — the public-channel equivalents (`channels:read`/`channels:history`)
don't cover private ones. Documented in `docs/04-extensions/integrations/slack.md`
and the README's setup steps; not enforced in code (same as every other
Slack scope — a missing scope surfaces as a real Slack API error at the
call site, not a pre-flight check this app performs).

### 2. What "사용자" (users) means — documentation, not a bug

Explained via a new paragraph in `docs/04-extensions/integrations/slack.md`
and the README's Slack walkthrough: the picker's second section adds a
person to `watched_user_ids` (a presence watch-list polled via
`users.getPresence`), completely unrelated to channels or DMs — selecting
a user does not open a DM channel, fetch messages, or send anything. Purely
a naming/labeling confusion, not a functional change.

### 3. Picker lists didn't scroll (`crates/ui/src/render.rs`)

Real bug. All three picker overlays (`render_slack_picker_overlay`,
`render_github_picker_overlay`, `render_calendar_picker_overlay`) built a
`Vec<ListItem>`, manually applied `Modifier::REVERSED` to whichever item
matched `picker.cursor`, and rendered via plain `frame.render_widget(List::new(items), area)`.
That call has no concept of a viewport — ratatui just draws `items`
starting at the top of the area and clips whatever doesn't fit. With
nothing tracking which page of a long list is currently visible, the
cursor could move to an index that was never drawn at all.

Fixed by switching all three to `ratatui::widgets::ListState` +
`frame.render_stateful_widget(...)`, with `ListState::default().with_selected(Some(index))`
built fresh each frame from the picker's own `cursor`. Ratatui's `List`
widget, when rendered statefully, adjusts the viewport on every render
call so the selected index is always visible — no persisted scroll-offset
field needed anywhere in `WorkspaceState`, since the offset is recomputed
from scratch (starting at 0) every frame and that recomputation alone is
enough to keep the selection on-screen.

Slack's picker needed one extra step: its `cursor` indexes the *logical*
channels-then-users list, but the rendered `Vec<ListItem>` also has two
bold section headers (and, for an empty channel list, a placeholder row)
interspersed — so the logical cursor and the rendered row index aren't
the same number. Tracked a `selected_render_index`, updated to the actual
`items.len()` at the moment the matching row is pushed, and passed *that*
to `ListState` instead of `picker.cursor` directly.

### 4. `j`/`k` no longer advertised as navigation, app-wide

Every remaining "j/k: 이동"-style hint (the Help overlay's "탐색" category,
and the three pickers' status-line hints) now says "↑/↓" instead — matching
the Calendar grid's existing arrow-only framing (`step26.md`/`step27.md`),
generalized to the rest of the app per this request. `docs/02-architecture/keyboard.md`
updated the same way. The underlying `j`/`k` key bindings are **left in
place** in the three pickers (and general panel navigation) — only the
*advertised* method changed, since removing the actual key wasn't asked
for here (unlike the Calendar grid, where removal was explicitly
requested in `step26.md`/`step27.md`). Existing muscle memory built on the
old hint still works; new users are only ever told about arrow keys.

## Verification

- `github_picker_scrolls_the_viewport_to_keep_a_faroff_cursor_visible`: a
  30-item list with the cursor at index 25 still renders that row's label
  somewhere in the buffer.
- `slack_picker_scrolls_to_a_faroff_cursor_in_the_users_section`: 20
  channels + a cursor 5 rows into the *users* section (logical index 25)
  still renders that specific user's label — proves the header-offset
  index translation is correct, not just that scrolling happens at all.
- `fetch_channel_list`'s live-network code isn't unit tested (consistent
  with every other live HTTP call in this codebase — no mock server
  dependency exists here); the `types` fix is a one-line literal change
  with no separate pure function to test it against. Manual verification
  with a real private channel is the acceptance check.
- `cargo fmt --all --check` / `cargo check --workspace --all-targets` /
  `cargo clippy --workspace --all-targets -- -D warnings` /
  `cargo test --workspace` all green. `crates/ui`: 163 tests (up from 161).
