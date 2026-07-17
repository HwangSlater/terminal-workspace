# Scheduler Specification

This document details the Scheduler Bounded Context, which governs reminders, timers, local agenda logging, and Pomodoro trackers.

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
