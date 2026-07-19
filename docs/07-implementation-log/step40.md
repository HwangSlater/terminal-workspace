# Implementation Plan - Phase 40: Color-code overlay borders by family

Requested directly: "도움말 같은 미니 창 뜨는거 색을 다르게 해서 구분하면
좋겠어" (make popups like the help overlay use different colors so they're
distinguishable). Skips the Decisions/`AskUserQuestion` cycle — a direct
visual request with one reasonable interpretation (every popup currently
shares one border color; give each family its own), not a tradeoff to
weigh.

## Context

Every overlay in this app — Help, Slack setup/picker, GitHub
setup/picker, Calendar setup/picker/rename/grid, Log Viewer — rendered
its popup border in the same `theme::ACCENT` (Nord8, cyan). With several
of these reachable in quick succession (e.g. `Ctrl+S` then `Ctrl+P`),
the only way to tell which kind of popup is currently open is reading
its title text; the border itself carried no information.

## Decision

Group the ten overlays into five families and give each its own border
color from the existing Nord palette (`crates/ui/src/theme.rs`), reusing
already-named semantic constants where a reasonable thematic fit exists
rather than inventing a same-colored duplicate:

| Family | Overlays | Color |
| :--- | :--- | :--- |
| Help | `?` | `theme::HELP` (new, Nord15 purple) |
| Slack | `Ctrl+S` setup, `Ctrl+P` picker | `theme::SLACK` (new, Nord10 blue) |
| GitHub | `Ctrl+G` setup, `Ctrl+R` picker | `theme::TEXT_BRIGHT` (existing, Nord6 near-white — GitHub's own brand is black-and-white, a neutral fit, no new constant needed) |
| Calendar | `Ctrl+L` setup, `Ctrl+K` picker, rename, `Ctrl+M` grid | `theme::SUCCESS` (existing, Nord14 green — a common calendar/agenda color, no new constant needed) |
| Log Viewer | `Ctrl+c` | `theme::LOG` (new, Nord12 orange) |

`NORD10`/`NORD12`/`NORD15` were already named in the palette but had no
call site (`#[allow(dead_code)]`, `step30.md`'s "name the whole palette
even if unused yet" convention) — the three new constants finally give
them one, so those `#[allow(dead_code)]` attributes were removed rather
than left stale.

The plain body-panel docks (Notification/Calendar, `dock_block`) are
**not** touched — they keep their existing focus-dependent `ACCENT`/plain
border (`Style::default()` when unfocused), unrelated to this change,
which is scoped to floating overlay popups specifically.

## Implementation

- `crates/ui/src/theme.rs`: added `HELP`, `SLACK`, `LOG` constants;
  removed the now-inaccurate `#[allow(dead_code)]` on `NORD10`/`NORD12`/
  `NORD15`.
- `crates/ui/src/render.rs`: all 11 `.border_style(Style::default().fg(theme::ACCENT))`
  call sites across the ten overlay-rendering functions updated to the
  matching family color above. Category/section title text *inside* each
  overlay (e.g. Help's "탐색"/"Slack 연동" category headers) is unchanged
  — this phase is about telling overlays apart from each other via their
  border, not restyling content within one.

## Verification

- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --
  -D warnings` / `cargo test --workspace` all green (189 `ui` tests, up
  from 188).
- New test, `crates/ui/src/render.rs`:
  `overlay_border_colors_differ_by_overlay_family` — calls
  `render_help_overlay`/`render_slack_setup_overlay`/`render_log_viewer_overlay`
  directly on their own isolated buffers (bypassing the full `render()`
  dispatch entirely, not just avoiding it for convenience) rather than
  going through the whole dashboard: a body panel's own `ACCENT` border
  could otherwise share a row with a centered popup and be picked up by
  a naive text search instead of the popup's actual border, since
  `Block`'s title text renders in its own unstyled default regardless of
  `border_style` and can't be used as a reliable color-bearing marker.
  Asserts each sampled family's border matches its documented `theme`
  constant and that all three sampled families are pairwise distinct
  colors.
- Manually ran the app: opened Help, Slack setup, GitHub picker, Calendar
  grid, and the Log Viewer in sequence, confirmed each has a visibly
  different border color and the same family's overlays (e.g. Slack
  setup vs. Slack picker) match each other.
