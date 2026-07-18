# Scheduler Specification

This document details the Scheduler Bounded Context, which governs reminders, timers, local agenda logging, and Pomodoro trackers.

> **Implementation Status**: **nothing on this page is built.** `crates/scheduler/src/lib.rs` is a ~25-line stub: an `AgendaScheduler` struct holding an unused `event_bus` handle (`#[allow(dead_code)]`) and a single `run_loop(&self) -> Result<()>` method that only logs `"Scheduler time loop started."` and returns immediately — no deadline computation, no sleep loop, no persistence. None of `SchedulerEvent`/`TriggerPolicy`/`PomodoroState`/`RecurrenceRule` exist, and the `workspace.redb` `scheduler_events` table this page describes was never created (also worth noting: this page predates ADR-0014's SQLite→`redb` switch, so its data-layer assumption is doubly stale). `AgendaScheduler` is never constructed anywhere outside its own crate — not wired into `crates/app/src/main.rs`, no TUI panel. This is not on the v1.0.0 release scope (`product-requirements.md` §4); treat this whole document as a future-phase design sketch, not current behavior.

---

## 1. Domain Entities & Value Objects

```rust
pub struct SchedulerEvent {
    pub id: EventId,
    pub title: String,
    pub start_time: EpochMs,
    pub duration: Minutes,
    pub trigger_policy: TriggerPolicy,
    pub is_recurring: bool,
    pub recur_rule: Option<RecurrenceRule>,
}

pub enum TriggerPolicy {
    TuiPopup,
    SlackWebhookAlert,
    TerminalBell,
}

pub struct PomodoroState {
    pub mode: PomodoroMode, // Work, ShortBreak, LongBreak
    pub remaining_seconds: u32,
    pub session_count: u32,
    pub is_running: bool,
}
```

---

## 2. Timer Event Loop Design

The Scheduler manages a background task running on a Tokio loop that sleeps until the next chronological event:

```text
       Scheduler Init (Query SQLite for events)
                  │
                  ▼
         [Compute Next Deadline]
                  │
                  ▼
         [Sleep Until Deadline]
                  │
                  ▼
       [Deadline Reached] ──(Raise Alarm)──> [Publish Notification Event]
                  │
                  ▼
         (Update Recurring Dates)
```
- The scheduler runs strictly in-memory once loaded, writing schedules to `workspace.redb` under the `scheduler_events` table for cold-starts.
