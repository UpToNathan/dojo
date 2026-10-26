//! What the display manager does next: greeter, session, greeter again. Pure. The service
//! reports what happened and carries out the [`Order`]s that come back.
//!
//! A greeter that cannot start is the one failure this module owns: three failed starts
//! within thirty seconds end the service with a non-zero status, so systemd's own start
//! limit fires and `tty2` stays the console (the unit sets `StartLimitBurst=3` over
//! `StartLimitIntervalSec=30` to match).

use std::collections::VecDeque;
use std::time::Duration;

pub const MAX_FAILURES: usize = 3;
pub const WINDOW: Duration = Duration::from_secs(30);
/// `EX_UNAVAILABLE`: the greeter, a service the login screen needs, is not working.
pub const EXIT_GREETER_UNAVAILABLE: i32 = 69;

/// Recent greeter failures, as times on any monotonic clock the caller keeps.
#[derive(Clone, Debug, Default)]
pub struct FailureWindow {
    failures: VecDeque<Duration>,
}

impl FailureWindow {
    /// Records a failure and says whether that makes [`MAX_FAILURES`] inside [`WINDOW`].
    pub fn record(&mut self, now: Duration) -> bool {
        while self
            .failures
            .front()
            .is_some_and(|first| now.saturating_sub(*first) >= WINDOW)
        {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
        self.failures.len() >= MAX_FAILURES
    }

    pub fn clear(&mut self) {
        self.failures.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// The greeter is starting or serving.
    Greeter,
    /// A user session was started from the greeter and has not ended.
    Session,
    Stopped,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    /// The greeter did not start, did not connect, or went away before it handed over.
    GreeterFailed { at: Duration },
    /// The greeter asked for a session that is going to start.
    SessionStarting,
    /// The session ended, or never started. Either way the greeter comes back.
    SessionOver,
    /// SIGTERM or SIGINT.
    Terminate,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    LaunchGreeter,
    StopGreeter,
    /// Refresh the look snapshot of the user in the session, or else of the last user.
    Snapshot,
    Exit(i32),
}

#[derive(Debug)]
pub struct Cycle {
    phase: Phase,
    failures: FailureWindow,
}

impl Cycle {
    /// A fresh service: the last user's look is taken again (their monitors or wallpaper may have
    /// changed since the session that last ended, or the snapshot predates a piece), then the greeter.
    pub fn start() -> (Cycle, Vec<Order>) {
        let cycle = Cycle {
            phase: Phase::Greeter,
            failures: FailureWindow::default(),
        };
        (cycle, vec![Order::Snapshot, Order::LaunchGreeter])
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// A greeter failed since the last session was handed over. The next greeter is not
    /// shown what the last one may have died of.
    pub fn greeter_failed_recently(&self) -> bool {
        !self.failures.is_empty()
    }

    pub fn on(&mut self, input: Input) -> Vec<Order> {
        match (self.phase, input) {
            (Phase::Stopped, _) => Vec::new(),
            (_, Input::Terminate) => {
                let was = std::mem::replace(&mut self.phase, Phase::Stopped);
                let mut orders = Vec::new();
                if was == Phase::Greeter {
                    orders.push(Order::StopGreeter);
                }
                orders.extend([Order::Snapshot, Order::Exit(0)]);
                orders
            }
            (Phase::Greeter, Input::GreeterFailed { at }) => {
                if self.failures.record(at) {
                    self.phase = Phase::Stopped;
                    vec![Order::Exit(EXIT_GREETER_UNAVAILABLE)]
                } else {
                    vec![Order::LaunchGreeter]
                }
            }
            (Phase::Greeter, Input::SessionStarting) => {
                self.phase = Phase::Session;
                self.failures.clear();
                Vec::new()
            }
            (Phase::Session, Input::SessionOver) => {
                self.phase = Phase::Greeter;
                vec![Order::Snapshot, Order::LaunchGreeter]
            }
            // The greeter is gone while a session runs, or a session ends with no greeter.
            (Phase::Session, Input::GreeterFailed { .. } | Input::SessionStarting)
            | (Phase::Greeter, Input::SessionOver) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> Input {
        Input::GreeterFailed {
            at: Duration::from_secs(seconds),
        }
    }

    #[test]
    fn the_first_orders_refresh_the_last_users_look_and_start_the_greeter() {
        let (cycle, orders) = Cycle::start();
        assert_eq!(orders, [Order::Snapshot, Order::LaunchGreeter]);
        assert_eq!(cycle.phase(), Phase::Greeter);
    }

    #[test]
    fn three_failed_starts_in_thirty_seconds_end_the_service() {
        let (mut cycle, _) = Cycle::start();
        assert_eq!(cycle.on(at(100)), [Order::LaunchGreeter]);
        assert_eq!(cycle.on(at(110)), [Order::LaunchGreeter]);
        assert_eq!(cycle.on(at(129)), [Order::Exit(EXIT_GREETER_UNAVAILABLE)]);
        assert_eq!(cycle.phase(), Phase::Stopped);
        assert_eq!(cycle.on(at(130)), [], "nothing after the end");
    }

    #[test]
    fn failures_older_than_the_window_are_forgotten() {
        let (mut cycle, _) = Cycle::start();
        assert_eq!(cycle.on(at(0)), [Order::LaunchGreeter]);
        assert_eq!(cycle.on(at(15)), [Order::LaunchGreeter]);
        assert_eq!(
            cycle.on(at(30)),
            [Order::LaunchGreeter],
            "the first has aged out"
        );
        assert_eq!(cycle.on(at(45)), [Order::LaunchGreeter]);
        assert_eq!(cycle.on(at(59)), [Order::Exit(EXIT_GREETER_UNAVAILABLE)]);
    }

    #[test]
    fn the_window_is_the_unit_start_limit() {
        let unit = include_str!("../../system-config/systemd/system/dojo-dm.service");
        assert!(unit.contains(&format!("StartLimitIntervalSec={}", WINDOW.as_secs())));
        assert!(unit.contains(&format!("StartLimitBurst={MAX_FAILURES}")));
    }

    #[test]
    fn a_session_resets_the_count_and_brings_the_greeter_back() {
        let (mut cycle, _) = Cycle::start();
        cycle.on(at(1));
        cycle.on(at(2));
        assert_eq!(cycle.on(Input::SessionStarting), []);
        assert_eq!(cycle.phase(), Phase::Session);
        assert_eq!(
            cycle.on(at(3)),
            [],
            "a dead greeter does not matter during a session"
        );
        assert_eq!(
            cycle.on(Input::SessionOver),
            [Order::Snapshot, Order::LaunchGreeter]
        );
        assert_eq!(cycle.on(at(4)), [Order::LaunchGreeter]);
        assert_eq!(cycle.on(at(5)), [Order::LaunchGreeter]);
        assert_eq!(cycle.on(at(6)), [Order::Exit(EXIT_GREETER_UNAVAILABLE)]);
    }

    #[test]
    fn a_failed_greeter_is_remembered_until_a_session_takes_over() {
        let (mut cycle, _) = Cycle::start();
        assert!(!cycle.greeter_failed_recently());
        cycle.on(at(1));
        assert!(cycle.greeter_failed_recently());
        cycle.on(Input::SessionStarting);
        assert!(!cycle.greeter_failed_recently());
    }

    #[test]
    fn terminate_stops_the_greeter_snapshots_and_exits_cleanly() {
        let (mut cycle, _) = Cycle::start();
        assert_eq!(
            cycle.on(Input::Terminate),
            [Order::StopGreeter, Order::Snapshot, Order::Exit(0)]
        );
        assert_eq!(cycle.on(Input::Terminate), []);
    }

    #[test]
    fn terminate_during_a_session_leaves_the_session_alone() {
        let (mut cycle, _) = Cycle::start();
        cycle.on(Input::SessionStarting);
        assert_eq!(
            cycle.on(Input::Terminate),
            [Order::Snapshot, Order::Exit(0)]
        );
        assert_eq!(cycle.phase(), Phase::Stopped);
    }

    #[test]
    fn a_session_that_never_started_is_not_a_greeter_failure() {
        let (mut cycle, _) = Cycle::start();
        for _ in 0..10 {
            assert_eq!(cycle.on(Input::SessionStarting), []);
            assert_eq!(
                cycle.on(Input::SessionOver),
                [Order::Snapshot, Order::LaunchGreeter]
            );
        }
        assert_eq!(cycle.phase(), Phase::Greeter);
    }
}
