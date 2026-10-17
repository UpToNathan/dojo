//! The lock screen's power timeline as one table. Every power timing constant lives here.

use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PowerPhase {
    /// Full brightness after activity.
    Active,
    /// One static dim frame, then nothing until the minute tick.
    Settled,
    /// The display is off: no frames, no minute tick, sensor released.
    DisplayOff,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Display {
    On,
    Off,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sensor {
    Armed,
    Disarmed,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PowerRules {
    /// Brightness of the lock screen; 1 is full.
    pub dim: f32,
    pub display: Display,
    pub minute_tick: bool,
    pub sensor: Sensor,
    /// How long this phase lasts without activity, and what follows it.
    pub next: Option<(Duration, PowerPhase)>,
}

const SETTLE_AFTER: Duration = Duration::from_secs(5);
const DISPLAY_OFF_AFTER: Duration = Duration::from_secs(30);

pub const fn rules(phase: PowerPhase) -> PowerRules {
    match phase {
        PowerPhase::Active => PowerRules {
            dim: 1.,
            display: Display::On,
            minute_tick: true,
            sensor: Sensor::Armed,
            next: Some((SETTLE_AFTER, PowerPhase::Settled)),
        },
        PowerPhase::Settled => PowerRules {
            dim: 0.45,
            display: Display::On,
            minute_tick: true,
            sensor: Sensor::Armed,
            next: Some((DISPLAY_OFF_AFTER, PowerPhase::DisplayOff)),
        },
        PowerPhase::DisplayOff => PowerRules {
            dim: 0.,
            display: Display::Off,
            minute_tick: false,
            sensor: Sensor::Disarmed,
            next: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_timeline_ends_dark_and_every_dark_phase_releases_the_sensor_and_the_tick() {
        let mut phase = PowerPhase::Active;
        let mut seen = vec![phase];
        while let Some((_, next)) = rules(phase).next {
            phase = next;
            seen.push(phase);
        }
        assert_eq!(
            seen,
            [
                PowerPhase::Active,
                PowerPhase::Settled,
                PowerPhase::DisplayOff
            ]
        );
        for phase in seen {
            let rule = rules(phase);
            if rule.display == Display::Off {
                assert_eq!((rule.sensor, rule.minute_tick), (Sensor::Disarmed, false));
            }
        }
    }
}
