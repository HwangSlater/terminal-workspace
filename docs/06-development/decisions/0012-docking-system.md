# ADR 0012: UI View Docking System

## Context
Standard terminal layouts are static grid structures. We need a way for plugins and core features to dynamically display viewports (e.g. Pomodoro widgets, Git diff viewers) without breaking active console rendering grids.

---

## Decision
We select a **Docking System Layout** rather than a dynamic custom layout engine.
- **Dock slots**: `Left`, `Center`, `Right`, `Bottom`.
- **Behavior**: Panels register to a specific slot. Multiple panels in the same slot are rendered as tab views, letting the user toggle focus using standard hotkeys.

---

## Alternatives Considered

### Custom Dynamic Layout Engine (Tree-based)
- **Pros**: Ultimate layout flexibility (users can resize any coordinate dynamically).
- **Cons**: Over-complicated math and keystroke routing logic for terminal emulators. High risk of rendering bugs. (Rejected).

---

## Consequences
- **Plugin Friendliness**: Plugins call `register_panel("pomodoro", DockSlot::Bottom)` and the host handles layout grids calculation.
- **Predictability**: Fixed docks keep the workspace layout clean and consistent.
