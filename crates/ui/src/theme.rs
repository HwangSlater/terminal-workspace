//! Nord color palette and shared visual primitives (`step30.md`) — one
//! fixed, hardcoded design, not a selectable theme. `config.toml`'s
//! dormant `theme` field and `docs/02-architecture/theme.md`'s theme list
//! stay aspirational; this module is the one design this app actually
//! ships, deliberately not gated behind either of them (`step30.md`
//! Decision 1 — "pour the effort into one best design" was chosen over
//! building a theme switcher).
//!
//! True 24-bit RGB throughout (`Color::Rgb`), not named
//! `ratatui::style::Color` variants (`Color::Cyan`, `Color::Red`, ...),
//! which resolve to whatever the user's own terminal color scheme maps
//! them to — the whole point of "one unified palette" is to *not* depend
//! on that. Needs a truecolor-capable terminal, which is virtually
//! universal today (Windows Terminal, iTerm2, GNOME Terminal/VTE,
//! Alacritty, kitty, Ghostty, ...); an older terminal without truecolor
//! support will show approximated colors, not crash.

use ratatui::style::{Color, Modifier, Style};

// Nord's 16 named swatches (<https://www.nordtheme.com/>), by their
// official index. Not all 16 have a call site yet, but naming them all
// keeps the palette self-documenting rather than inventing semantic names
// for colors nothing uses.
#[allow(dead_code)] // part of the named palette; no call site needs a background swatch yet
pub const NORD0: Color = Color::Rgb(46, 52, 64);
#[allow(dead_code)] // part of the named palette; no call site needs a background swatch yet
pub const NORD1: Color = Color::Rgb(59, 66, 82);
pub const NORD2: Color = Color::Rgb(67, 76, 94);
pub const NORD3: Color = Color::Rgb(76, 86, 106);
#[allow(dead_code)] // part of the named palette; no call site needs nord4 specifically yet
pub const NORD4: Color = Color::Rgb(216, 222, 233);
#[allow(dead_code)] // part of the named palette; no call site needs nord5 specifically yet
pub const NORD5: Color = Color::Rgb(229, 233, 240);
pub const NORD6: Color = Color::Rgb(236, 239, 244);
pub const NORD7: Color = Color::Rgb(143, 188, 187);
pub const NORD8: Color = Color::Rgb(136, 192, 208);
pub const NORD9: Color = Color::Rgb(129, 161, 193);
#[allow(dead_code)] // part of the named palette; no call site needs nord10 specifically yet
pub const NORD10: Color = Color::Rgb(94, 129, 172);
pub const NORD11: Color = Color::Rgb(191, 97, 106);
#[allow(dead_code)] // part of the named palette; no call site needs nord12 specifically yet
pub const NORD12: Color = Color::Rgb(208, 135, 112);
pub const NORD13: Color = Color::Rgb(235, 203, 139);
pub const NORD14: Color = Color::Rgb(163, 190, 140);
#[allow(dead_code)] // part of the named palette; no call site needs nord15 specifically yet
pub const NORD15: Color = Color::Rgb(180, 142, 173);

/// Focused borders, primary interactive accents (block titles, help
/// category headers).
pub const ACCENT: Color = NORD8;
/// A brighter accent for things that need to stand out even against
/// `ACCENT`-colored chrome around them (the Calendar grid's "today").
pub const ACCENT_BRIGHT: Color = NORD7;
/// Dimmed/secondary text — timestamps, status-line hints, "no data" empty
/// states, low-priority/offline/disconnected.
pub const MUTED: Color = NORD3;
/// Brightest text — selected-row foreground.
pub const TEXT_BRIGHT: Color = NORD6;
/// Connected / success / low priority / active presence.
pub const SUCCESS: Color = NORD14;
/// Connecting-in-progress / medium-severity / has-events marker.
pub const WARNING: Color = NORD13;
/// Failed / high priority / Sunday.
pub const ERROR: Color = NORD11;
/// A secondary accent distinct from `ACCENT` — Saturday's weekend color.
pub const INFO: Color = NORD9;
/// Selected-row background (`step30.md` Decision 2 — replaces bare
/// `Modifier::REVERSED`, which just swaps whatever fg/bg a cell already
/// has rather than using this palette).
pub const SELECTED_BG: Color = NORD2;

/// A list row's "the cursor is here" style — an explicit highlight color
/// instead of `Modifier::REVERSED` (`step30.md` Decision 2), so selection
/// reads as part of the same designed palette rather than inverting
/// whatever's underneath it.
pub fn selected_style() -> Style {
    Style::default()
        .bg(SELECTED_BG)
        .fg(TEXT_BRIGHT)
        .add_modifier(Modifier::BOLD)
}

/// The classic "dots" CLI spinner (10 Braille frames) — the same shape
/// most terminal tools (`cargo`, `npm`, ...) use, just implemented by
/// hand since this app has no spinner crate dependency yet and one frame
/// glyph per tick is all that's needed.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The spinner glyph for a given animation tick (`WorkspaceState::anim_tick`,
/// `step30.md` Decision 3) — wraps around `SPINNER_FRAMES`, so any
/// ever-increasing tick counter works with no bounds-checking at the call
/// site.
#[must_use]
pub fn spinner_frame(tick: u64) -> &'static str {
    SPINNER_FRAMES[(tick as usize) % SPINNER_FRAMES.len()]
}

/// A block-character progress bar (`step30.md` Decision 4), e.g.
/// `"████░░░░"` at `ratio = 0.5, width = 8`. `ratio` is clamped to
/// `[0.0, 1.0]` — a caller passing an out-of-range ratio (e.g. a stale
/// `remaining_secs` briefly exceeding `total_secs` right after a mode
/// switch) gets a full or empty bar instead of a panic or a garbled
/// string.
#[must_use]
pub fn progress_bar(ratio: f64, width: usize) -> String {
    let ratio = ratio.clamp(0.0, 1.0);
    // Rounds rather than truncates so a ratio just under a whole block
    // boundary (e.g. 0.99 at width 10) still reads as "almost full"
    // instead of visually lagging a full block behind the real fraction.
    #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
    let filled = ((ratio * width as f64).round() as usize).min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_frame_cycles_through_all_ten_frames_before_repeating() {
        let first_ten: Vec<&str> = (0..10).map(spinner_frame).collect();
        assert_eq!(first_ten, SPINNER_FRAMES.to_vec());
        // Wraps around, not panics or produces something new.
        assert_eq!(spinner_frame(10), spinner_frame(0));
        assert_eq!(spinner_frame(23), spinner_frame(3));
    }

    #[test]
    fn progress_bar_at_zero_is_all_empty() {
        assert_eq!(progress_bar(0.0, 10), "░".repeat(10));
    }

    #[test]
    fn progress_bar_at_one_is_all_filled() {
        assert_eq!(progress_bar(1.0, 10), "█".repeat(10));
    }

    #[test]
    fn progress_bar_at_half_splits_evenly() {
        assert_eq!(
            progress_bar(0.5, 10),
            format!("{}{}", "█".repeat(5), "░".repeat(5))
        );
    }

    #[test]
    fn progress_bar_clamps_an_out_of_range_ratio() {
        assert_eq!(progress_bar(-0.5, 4), "░".repeat(4));
        assert_eq!(progress_bar(1.5, 4), "█".repeat(4));
    }
}
