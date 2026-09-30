//! Frame pacing: when to start rendering so a frame is ready just before the vblank.
use std::time::Duration;

const SAMPLES: usize = 16;

/// The most recent render durations of one output.
#[derive(Default)]
pub struct RenderTimes {
    samples: [Duration; SAMPLES],
    next: usize,
    filled: usize,
}

impl RenderTimes {
    pub fn record(&mut self, took: Duration) {
        self.samples[self.next] = took;
        self.next = (self.next + 1) % SAMPLES;
        self.filled = (self.filled + 1).min(SAMPLES);
    }

    /// The slowest of the recent frames (zero before any was recorded).
    pub fn worst(&self) -> Duration {
        self.samples[..self.filled].iter().copied().max().unwrap_or_default()
    }

    pub fn average(&self) -> Duration {
        if self.filled == 0 {
            return Duration::ZERO;
        }
        self.samples[..self.filled].iter().sum::<Duration>() / self.filled as u32
    }
}

/// How long to wait from `now` before rendering, so the frame takes `cost + margin`
/// and is done at the next vblank. Vblanks happen at `last_vblank + n * interval`.
/// All times are on the same clock. Never waits longer than one interval.
pub fn late_delay(now: Duration, last_vblank: Duration, interval: Duration, cost: Duration, margin: Duration) -> Duration {
    if interval.is_zero() {
        return Duration::ZERO;
    }
    let since = now.saturating_sub(last_vblank);
    let n = since.as_nanos() / interval.as_nanos() + 1;
    let next_vblank = last_vblank + Duration::from_nanos((interval.as_nanos() * n) as u64);
    let start = next_vblank.saturating_sub(cost + margin);
    start.saturating_sub(now).min(interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn waits_until_render_fits_before_vblank() {
        // 10 ms interval, vblank at 100, now 101: next vblank 110, render 3 ms + 2 ms margin.
        assert_eq!(late_delay(101 * MS, 100 * MS, 10 * MS, 3 * MS, 2 * MS), 4 * MS);
    }

    #[test]
    fn renders_at_once_when_too_late() {
        assert_eq!(late_delay(108 * MS, 100 * MS, 10 * MS, 3 * MS, 2 * MS), Duration::ZERO);
    }

    #[test]
    fn extrapolates_over_missed_vblanks() {
        // Last known vblank is 3.5 intervals ago: the next is at 140.
        assert_eq!(late_delay(135 * MS, 100 * MS, 10 * MS, 1 * MS, 1 * MS), 3 * MS);
    }

    #[test]
    fn slow_render_never_waits_more_than_an_interval() {
        assert_eq!(late_delay(100 * MS, 100 * MS, 10 * MS, Duration::ZERO, Duration::ZERO), 10 * MS);
        assert_eq!(late_delay(100 * MS, 100 * MS, Duration::ZERO, MS, MS), Duration::ZERO);
    }

    #[test]
    fn tracks_recent_render_times() {
        let mut times = RenderTimes::default();
        assert_eq!(times.worst(), Duration::ZERO);
        times.record(2 * MS);
        times.record(6 * MS);
        assert_eq!(times.worst(), 6 * MS);
        assert_eq!(times.average(), 4 * MS);
        for _ in 0..SAMPLES {
            times.record(MS);
        }
        assert_eq!(times.worst(), MS);
    }
}
