# Scheduler Specification

This document details the Scheduler Bounded Context, which governs reminders, timers, local agenda logging, and Pomodoro trackers.

> **Implementation Status**: **the Pomodoro slice is real as of Phase 18** (`docs/07-implementation-log/step18.md`) — `AgendaScheduler` (`crates/scheduler/src/lib.rs`) is a working timer: computed-not-ticked `PomodoroState` (`Work`/`Break` modes, session count, running/paused), a cancellable `run_loop` background task (`tokio::select!` over a sleep and a `Notify` interrupt), auto-transition between Work/Break on natural completion with a terminal bell + `Event::SystemAlert`, and `/pomodoro start|pause|reset` command-bar control wired through `WorkspaceCommandHandler` into a header display in the TUI. **Everything else on this page remains unbuilt**: `SchedulerEvent`, `TriggerPolicy`, `RecurrenceRule`, generic (non-Pomodoro) reminders, and any persistence — the shipped `AgendaScheduler` holds Pomodoro state purely in memory (an `RwLock`, not `workspace.redb`) and doesn't survive a restart. `ShortBreak`/`LongBreak` variants described elsewhere on this page were also deliberately not built (see step18.md's Implementation Notes) — only a single `Work`/`Break` cycle exists. Treat the rest of this document as a future-phase design sketch, not current behavior.

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
