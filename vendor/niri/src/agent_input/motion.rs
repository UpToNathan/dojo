use std::time::Duration;

use smithay::utils::{Logical, Point};

/// A finite movement with zero final velocity and acceleration. Retargeting
/// carries the current velocity into the next movement.
pub(super) struct Motion {
    from: Point<f64, Logical>,
    to: Point<f64, Logical>,
    velocity: Point<f64, Logical>,
    start: Duration,
    duration: Duration,
}

impl Motion {
    pub fn new(
        from: Point<f64, Logical>,
        to: Point<f64, Logical>,
        velocity: Point<f64, Logical>,
        start: Duration,
    ) -> Self {
        let distance = (to.x - from.x).hypot(to.y - from.y);
        let seconds = (0.10 + distance.sqrt() * 0.012).clamp(0.12, 0.55);
        Self {
            from,
            to,
            velocity,
            start,
            duration: Duration::from_secs_f64(seconds),
        }
    }

    pub fn destination(&self) -> Point<f64, Logical> {
        self.to
    }

    pub fn remaining(&self, now: Duration) -> Duration {
        (self.start + self.duration).saturating_sub(now)
    }

    pub fn sample(&self, now: Duration) -> (Point<f64, Logical>, Point<f64, Logical>) {
        if self.remaining(now).is_zero() {
            return (self.to, (0., 0.).into());
        }
        let seconds = self.duration.as_secs_f64();
        let t = (now.saturating_sub(self.start).as_secs_f64() / seconds).clamp(0., 1.);
        // Pinned CUA minimum-jerk easing; local Hermite correction retains
        // redirected velocity without introducing overshoot/settle after arrival.
        let progress = cua_cursor_motion::ease::min_jerk(t);
        let tangent = t - 6. * t.powi(3) + 8. * t.powi(4) - 3. * t.powi(5);
        let speed = 30. * t.powi(2) * (1. - t).powi(2) / seconds;
        let tangent_speed = 1. - 18. * t.powi(2) + 32. * t.powi(3) - 15. * t.powi(4);
        let delta = self.to - self.from;
        (
            (
                self.from.x + delta.x * progress + self.velocity.x * seconds * tangent,
                self.from.y + delta.y * progress + self.velocity.y * seconds * tangent,
            )
                .into(),
            (
                delta.x * speed + self.velocity.x * tangent_speed,
                delta.y * speed + self.velocity.y * tangent_speed,
            )
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_and_finishes_at_rest_without_overshoot() {
        let motion = Motion::new(
            (100., 200.).into(),
            (1000., 500.).into(),
            (0., 0.).into(),
            Duration::ZERO,
        );
        assert_eq!(
            motion.sample(Duration::ZERO),
            ((100., 200.).into(), (0., 0.).into())
        );
        let mut previous = 100.;
        for ms in 1..600 {
            let (position, _) = motion.sample(Duration::from_millis(ms));
            assert!(position.x >= previous && position.x <= 1000.);
            assert!(position.y >= 200. && position.y <= 500.);
            previous = position.x;
        }
        assert_eq!(
            motion.sample(Duration::from_secs(1)),
            ((1000., 500.).into(), (0., 0.).into())
        );
    }

    #[test]
    fn retarget_preserves_position_and_velocity() {
        let motion = Motion::new(
            (0., 0.).into(),
            (900., 400.).into(),
            (0., 0.).into(),
            Duration::ZERO,
        );
        let now = Duration::from_millis(120);
        let (position, velocity) = motion.sample(now);
        let redirected = Motion::new(position, (50., 100.).into(), velocity, now);
        assert_eq!(redirected.sample(now), (position, velocity));
        assert_eq!(
            redirected.sample(now + Duration::from_secs(1)),
            ((50., 100.).into(), (0., 0.).into())
        );
    }

    #[test]
    fn duration_scales_with_distance_and_stays_bounded() {
        let movement = |x| {
            Motion::new(
                (0., 0.).into(),
                (x, 0.).into(),
                (0., 0.).into(),
                Duration::ZERO,
            )
        };
        assert!(movement(10.).duration < movement(1000.).duration);
        assert!(movement(0.).duration >= Duration::from_millis(120));
        assert!(movement(100000.).duration <= Duration::from_millis(550));
    }
}
