//! The 30-second default of the first-run mode choice. Shared by the windows: without a choice in time the chat starts
//! in Local mode, and nothing is saved. Any input stops the timer; the caller starts it again when the choice comes back.

use std::time::{Duration, Instant};

pub const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChoiceTimer {
    deadline: Option<Instant>,
}

impl ChoiceTimer {
    pub fn start(now: Instant) -> ChoiceTimer {
        ChoiceTimer { deadline: Some(now + TIMEOUT) }
    }

    /// The user is choosing now: no automatic start from here on.
    pub fn stop(&mut self) {
        self.deadline = None;
    }

    pub fn running(&self) -> bool {
        self.deadline.is_some()
    }

    /// True when the time is up and the timer was not stopped.
    pub fn timed_out(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }

    /// Whole seconds left, rounded up, while the timer runs; None once it has stopped.
    pub fn remaining(&self, now: Instant) -> Option<u64> {
        self.deadline.map(|deadline| deadline.saturating_duration_since(now).as_secs_f64().ceil() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirty_seconds_then_timed_out() {
        let start = Instant::now();
        let timer = ChoiceTimer::start(start);
        assert!(!timer.timed_out(start + Duration::from_secs(29)));
        assert!(timer.timed_out(start + TIMEOUT));
        assert_eq!(timer.remaining(start + Duration::from_secs(29)), Some(1));
        assert_eq!(timer.remaining(start), Some(30));
    }

    #[test]
    fn stopped_timer_never_times_out() {
        let start = Instant::now();
        let mut timer = ChoiceTimer::start(start);
        timer.stop();
        assert!(!timer.running());
        assert!(!timer.timed_out(start + TIMEOUT * 10));
        assert_eq!(timer.remaining(start), None);
    }
}
