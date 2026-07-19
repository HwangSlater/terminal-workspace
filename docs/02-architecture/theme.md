# Themes

> **Implementation Status (`step30.md`)**: `crates/ui/src/theme.rs` implements the **Nord** palette below as the app's one fixed, hardcoded visual design — every panel and overlay's colors, the selected-row highlight, the loading spinner, and the Pomodoro progress bar all draw from it. This list's other four entries (Light, Catppuccin, Tokyo Night, and Nord's own dark-vs-light variants) and `config.toml`'s `theme` field / `WorkspaceState.active_theme` remain aspirational — `step30.md` deliberately chose "pour the effort into one best design" over building a theme switcher. Revisit this doc if that changes.

## Nord (implemented)

True 24-bit RGB (`ratatui::style::Color::Rgb`), not terminal-theme-dependent named colors — see `crates/ui/src/theme.rs`'s module doc for why. Semantic roles, not raw swatch names, are what call sites actually use:

| Role | Nord swatch | Used for |
| :--- | :--- | :--- |
| `ACCENT` | nord8 | Focused borders, block titles, help category headers |
| `ACCENT_BRIGHT` | nord7 | Calendar grid's "today" marker |
| `MUTED` | nord3 | Timestamps, hints, empty states, low-priority/offline/disconnected |
| `TEXT_BRIGHT` | nord6 | Selected-row foreground |
| `SUCCESS` | nord14 | Connected, active presence, low-severity, running Pomodoro |
| `WARNING` | nord13 | Connecting/reconnecting, has-events marker, paused Pomodoro |
| `ERROR` | nord11 | Failed, high priority, Sunday |
| `INFO` | nord9 | Saturday |
| `SELECTED_BG` | nord2 | Selected-row background |

## Not yet built (aspirational)

Light
Catppuccin
Tokyo Night
