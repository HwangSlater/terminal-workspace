//! Scheduler Domain: a Pomodoro work/break timer (`step18.md`). Generic
//! recurring reminders/agenda logging (`docs/03-domain/scheduler.md`'s
//! broader sketch — `SchedulerEvent`, `RecurrenceRule`, multiple
//! `TriggerPolicy` variants) remain unbuilt; nothing needs them yet, and
//! Calendar already covers "remind me about a calendar event"
//! (`Event::CalendarReminderTriggered`, `step12.md`).

use common::Result;
use events::{Event, EventBus};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, RwLock};
use tokio::time::Instant;

/// Default work session length if `start` doesn't specify one.
pub const DEFAULT_WORK_MINUTES: u32 = 25;
/// Default break length if `start` doesn't specify one.
pub const DEFAULT_BREAK_MINUTES: u32 = 5;

/// Which phase of a Pomodoro cycle is active.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PomodoroMode {
    /// A focus session.
    #[default]
    Work,
    /// A rest session between Work sessions.
    Break,
}

/// Internal, mutable Pomodoro timer state (`step18.md` Decision 1) --
/// remaining time is derived from `started_at` at read time, never
/// decremented by a background tick.
#[derive(Debug, Clone)]
struct PomodoroState {
    mode: PomodoroMode,
    session_count: u32,
    is_running: bool,
    /// Distinguishes "never started" from "paused" (both have
    /// `is_running == false`) -- `PausePomodoro`/`ResetPomodoro` on a
    /// never-started timer is a real user-facing error, not a silent
    /// no-op.
    has_been_started: bool,
    /// When the current running stretch began. `None` while paused/idle.
    /// `tokio::time::Instant`, not `std::time::SystemTime` -- deliberately:
    /// monotonic (immune to real-world system clock adjustments), and
    /// critically, respects `tokio::time::pause()`/`advance()` the same
    /// way `tokio::time::sleep` (Decision 2's cancellable timer) does. A
    /// real bug was found and fixed here (`step18.md` Implementation
    /// Notes): with `SystemTime`, `elapsed_secs()` tracked real wall-clock
    /// time while the background sleep tracked tokio's virtual time under
    /// `#[tokio::test(start_paused = true)]`, so `on_session_ended`'s
    /// safety re-check (`remaining_secs() > 0`) always saw the *old* real
    /// elapsed time and silently aborted -- the trigger never fired.
    started_at: Option<Instant>,
    /// Seconds already elapsed before the current running stretch (i.e.
    /// accumulated across any prior pauses this session) -- added to the
    /// current stretch's elapsed time to get the session total.
    elapsed_before_pause_secs: u64,
    work_duration_secs: u64,
    break_duration_secs: u64,
}

impl Default for PomodoroState {
    fn default() -> Self {
        Self {
            mode: PomodoroMode::Work,
            session_count: 0,
            is_running: false,
            has_been_started: false,
            started_at: None,
            elapsed_before_pause_secs: 0,
            work_duration_secs: u64::from(DEFAULT_WORK_MINUTES) * 60,
            break_duration_secs: u64::from(DEFAULT_BREAK_MINUTES) * 60,
        }
    }
}

impl PomodoroState {
    fn current_duration_secs(&self) -> u64 {
        match self.mode {
            PomodoroMode::Work => self.work_duration_secs,
            PomodoroMode::Break => self.break_duration_secs,
        }
    }

    fn elapsed_secs(&self) -> u64 {
        let running_elapsed = if self.is_running {
            self.started_at
                .map_or(0, |t| Instant::now().duration_since(t).as_secs())
        } else {
            0
        };
        self.elapsed_before_pause_secs + running_elapsed
    }

    fn remaining_secs(&self) -> u64 {
        self.current_duration_secs()
            .saturating_sub(self.elapsed_secs())
    }

    fn start(&mut self, work_minutes: u32, break_minutes: u32) {
        self.mode = PomodoroMode::Work;
        self.work_duration_secs = u64::from(work_minutes.max(1)) * 60;
        self.break_duration_secs = u64::from(break_minutes.max(1)) * 60;
        self.session_count = 0;
        self.elapsed_before_pause_secs = 0;
        self.started_at = Some(Instant::now());
        self.is_running = true;
        self.has_been_started = true;
    }

    fn toggle_pause(&mut self) {
        if self.is_running {
            self.elapsed_before_pause_secs = self.elapsed_secs();
            self.is_running = false;
            self.started_at = None;
        } else {
            self.is_running = true;
            self.started_at = Some(Instant::now());
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A point-in-time, render-friendly snapshot of the timer
/// (`step18.md` Decision 1) -- what `crates/ui`'s header actually reads.
/// `Default` is the idle/never-started state (`has_been_started: false`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PomodoroSnapshot {
    /// Current phase.
    pub mode: PomodoroMode,
    /// Completed Work sessions since the last `start`/`reset`.
    pub session_count: u32,
    /// `true` if actively counting down (not paused, not idle).
    pub is_running: bool,
    /// `false` only if no session has ever been started (or it was reset).
    pub has_been_started: bool,
    /// Seconds left in the current phase.
    pub remaining_secs: u64,
    /// The current phase's total length in seconds (`work_duration_secs`/
    /// `break_duration_secs`) -- `step30.md`, for computing an elapsed
    /// ratio (the header's progress bar). Not derivable from
    /// `remaining_secs` alone once a session is underway.
    pub total_secs: u64,
}

/// Dynamic Agenda scheduler managing Pomodoro work/break cycles.
pub struct AgendaScheduler {
    event_bus: Arc<dyn EventBus>,
    state: RwLock<PomodoroState>,
    /// Wakes [`Self::run_loop`] whenever `start`/`toggle_pause`/`reset`
    /// changes the timer, so it recomputes its sleep target instead of
    /// firing a stale "session ended" trigger for a session that was
    /// paused or reset out from under it (`step18.md` Decision 2).
    /// Deliberately `notify_one`, not `notify_waiters` -- a real race was
    /// found and fixed here (`step18.md` Implementation Notes):
    /// `notify_waiters` only wakes tasks *already* awaiting `.notified()`,
    /// so a `start()` call landing before `run_loop`'s first poll would
    /// have been silently lost, leaving the loop waiting forever on a
    /// notification that already happened. `notify_one` buffers a single
    /// permit when nobody's listening yet, which `run_loop`'s next
    /// `.notified().await` immediately consumes -- correct regardless of
    /// which side reaches its await point first.
    interrupt: Notify,
}

impl AgendaScheduler {
    /// Create a new, idle scheduler. Nothing runs until [`Self::run_loop`]
    /// is spawned as a background task.
    #[must_use]
    pub fn new(event_bus: Arc<dyn EventBus>) -> Arc<Self> {
        Arc::new(Self {
            event_bus,
            state: RwLock::new(PomodoroState::default()),
            interrupt: Notify::new(),
        })
    }

    /// Start (or restart) a Pomodoro cycle from a fresh Work session.
    pub async fn start(&self, work_minutes: u32, break_minutes: u32) {
        let mut state = self.state.write().await;
        state.start(work_minutes, break_minutes);
        drop(state);
        self.interrupt.notify_one();
    }

    /// Toggle running/paused. Errors if no session has ever been started
    /// (`step18.md` Decision 4).
    pub async fn toggle_pause(&self) -> Result<()> {
        let mut state = self.state.write().await;
        if !state.has_been_started {
            return Err(common::WorkspaceError::Internal(
                "no active Pomodoro session -- start one first".to_string(),
            ));
        }
        state.toggle_pause();
        drop(state);
        self.interrupt.notify_one();
        Ok(())
    }

    /// Stop and clear the timer entirely, back to the never-started state.
    pub async fn reset(&self) {
        let mut state = self.state.write().await;
        state.reset();
        drop(state);
        self.interrupt.notify_one();
    }

    /// A fresh snapshot for display -- computed on demand, safe to call
    /// once per render frame (`step18.md` Decision 1).
    pub async fn snapshot(&self) -> PomodoroSnapshot {
        let state = self.state.read().await;
        PomodoroSnapshot {
            mode: state.mode,
            session_count: state.session_count,
            is_running: state.is_running,
            has_been_started: state.has_been_started,
            remaining_secs: state.remaining_secs(),
            total_secs: state.current_duration_secs(),
        }
    }

    /// Spawns background loop evaluating alarm deadlines. Never returns
    /// under normal operation; intended to run for the process lifetime
    /// as a single background task (`crates/app/src/main.rs`).
    pub async fn run_loop(&self) {
        tracing::info!("Scheduler run loop started.");
        loop {
            let sleep_target = {
                let state = self.state.read().await;
                state
                    .is_running
                    .then(|| Duration::from_secs(state.remaining_secs()))
            };

            match sleep_target {
                Some(duration) => {
                    tokio::select! {
                        () = tokio::time::sleep(duration) => {
                            self.on_session_ended().await;
                        }
                        () = self.interrupt.notified() => {
                            // State changed externally (pause/reset/start)
                            // -- loop back and recompute the sleep target
                            // instead of firing a stale trigger.
                        }
                    }
                }
                None => self.interrupt.notified().await,
            }
        }
    }

    /// Auto-transitions Work -> Break -> Work (incrementing
    /// `session_count` on every completed Work session) and fires the
    /// trigger (`step18.md` Decision 3): a terminal bell plus a reused
    /// `Event::SystemAlert` -- `Event` is frozen by Architecture Freeze v1
    /// (`docs/06-development/development.md` §3), so this reuses the
    /// existing generic variant rather than adding a new one.
    async fn on_session_ended(&self) {
        let message = {
            let mut state = self.state.write().await;
            // Re-check under the write lock: guards a race where the sleep
            // fired concurrently with an external change that already
            // grabbed the lock first (e.g. a reset landing just before
            // this task woke up).
            if !state.is_running || state.remaining_secs() > 0 {
                return;
            }
            let message = match state.mode {
                PomodoroMode::Work => {
                    state.session_count += 1;
                    state.mode = PomodoroMode::Break;
                    "Work session complete -- take a break!".to_string()
                }
                PomodoroMode::Break => {
                    state.mode = PomodoroMode::Work;
                    "Break's over -- back to work!".to_string()
                }
            };
            state.elapsed_before_pause_secs = 0;
            state.started_at = Some(Instant::now());
            message
        };

        tracing::info!("{message}");
        print!("\x07");
        let _ = std::io::stdout().flush();
        let _ = self.event_bus.publish(Event::SystemAlert(message)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use events::InProcessEventBus;
    use std::time::Duration as StdDuration;
    use tokio::time::{advance, timeout};

    fn state_with_elapsed(elapsed_secs: u64, duration_secs: u64) -> PomodoroState {
        PomodoroState {
            mode: PomodoroMode::Work,
            session_count: 0,
            is_running: true,
            has_been_started: true,
            started_at: Some(Instant::now() - StdDuration::from_secs(elapsed_secs)),
            elapsed_before_pause_secs: 0,
            work_duration_secs: duration_secs,
            break_duration_secs: u64::from(DEFAULT_BREAK_MINUTES) * 60,
        }
    }

    #[test]
    fn remaining_secs_is_duration_minus_elapsed() {
        let state = state_with_elapsed(30, 100);
        assert_eq!(state.remaining_secs(), 70);
    }

    #[test]
    fn remaining_secs_clamps_to_zero_past_the_deadline() {
        let state = state_with_elapsed(150, 100);
        assert_eq!(state.remaining_secs(), 0);
    }

    #[test]
    fn a_never_started_timer_reports_the_full_default_work_duration() {
        let state = PomodoroState::default();
        assert_eq!(state.remaining_secs(), state.work_duration_secs);
        assert!(!state.has_been_started);
    }

    #[test]
    fn pausing_freezes_remaining_time_and_resuming_continues_from_there() {
        let mut state = state_with_elapsed(30, 100);
        state.toggle_pause(); // pause
        let remaining_at_pause = state.remaining_secs();
        assert_eq!(remaining_at_pause, 70);

        // Remaining time must not keep draining while paused.
        std::thread::sleep(StdDuration::from_millis(10));
        assert_eq!(state.remaining_secs(), remaining_at_pause);

        state.toggle_pause(); // resume
        assert!(state.is_running);
        assert_eq!(state.remaining_secs(), remaining_at_pause);
    }

    #[tokio::test]
    async fn toggle_pause_on_a_never_started_scheduler_is_a_real_error() {
        let event_bus = Arc::new(InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let scheduler = AgendaScheduler::new(event_bus);
        assert!(scheduler.toggle_pause().await.is_err());
    }

    #[tokio::test]
    async fn snapshot_reflects_a_started_session() {
        let event_bus = Arc::new(InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let scheduler = AgendaScheduler::new(event_bus);
        scheduler.start(25, 5).await;

        let snapshot = scheduler.snapshot().await;
        assert!(snapshot.has_been_started);
        assert!(snapshot.is_running);
        assert_eq!(snapshot.mode, PomodoroMode::Work);
        assert_eq!(snapshot.remaining_secs, 25 * 60);
        assert_eq!(snapshot.total_secs, 25 * 60);
    }

    #[tokio::test]
    async fn total_secs_stays_the_phase_length_as_remaining_secs_counts_down() {
        // `step30.md` -- `total_secs` is the header progress bar's
        // denominator; it must not itself decrease as time passes, or
        // every ratio computed from it would be wrong.
        let event_bus = Arc::new(InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let scheduler = AgendaScheduler::new(event_bus);
        scheduler.start(25, 5).await;
        tokio::time::sleep(Duration::from_millis(10)).await;

        let snapshot = scheduler.snapshot().await;
        assert_eq!(snapshot.total_secs, 25 * 60);
        assert!(snapshot.remaining_secs <= snapshot.total_secs);
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_ending_naturally_publishes_system_alert_and_advances_the_mode() {
        let event_bus = Arc::new(InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let mut event_rx = event_bus.subscribe();
        let scheduler = AgendaScheduler::new(Arc::clone(&event_bus) as Arc<dyn EventBus>);

        let loop_scheduler = Arc::clone(&scheduler);
        tokio::spawn(async move { loop_scheduler.run_loop().await });
        tokio::task::yield_now().await;

        // 1-second "minute" isn't available in the public API (minutes
        // only) -- use the smallest real unit, 1 minute, and advance
        // virtual time past it (`start_paused = true` -- no real waiting).
        scheduler.start(1, 5).await;
        tokio::task::yield_now().await;
        advance(StdDuration::from_secs(61)).await;
        tokio::task::yield_now().await;

        let event = timeout(StdDuration::from_millis(200), event_rx.recv())
            .await
            .expect("a SystemAlert should have been published")
            .expect("the event bus should not have closed");
        assert!(matches!(event, Event::SystemAlert(msg) if msg.contains("break")));

        let snapshot = scheduler.snapshot().await;
        assert_eq!(snapshot.mode, PomodoroMode::Break);
        assert_eq!(snapshot.session_count, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn resetting_before_a_session_ends_prevents_the_stale_trigger_from_firing() {
        // The single most important test in this phase (`step18.md`
        // Verification Plan): proves the cancellation mechanism actually
        // works, not just that the happy path publishes an event.
        let event_bus = Arc::new(InProcessEventBus::new(16)) as Arc<dyn EventBus>;
        let mut event_rx = event_bus.subscribe();
        let scheduler = AgendaScheduler::new(Arc::clone(&event_bus) as Arc<dyn EventBus>);

        let loop_scheduler = Arc::clone(&scheduler);
        tokio::spawn(async move { loop_scheduler.run_loop().await });

        scheduler.start(1, 5).await;
        advance(StdDuration::from_secs(10)).await; // partway through
        scheduler.reset().await;
        advance(StdDuration::from_secs(120)).await; // past the original 60s deadline

        let result = timeout(StdDuration::from_millis(200), event_rx.recv()).await;
        assert!(
            result.is_err(),
            "expected no SystemAlert after reset, but one was received: {result:?}"
        );

        let snapshot = scheduler.snapshot().await;
        assert!(!snapshot.has_been_started);
    }
}
