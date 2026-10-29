//! The output clock: a ticker with absolute deadlines from the tick index
//! (`FrameRate::tick_offset_ns`), so the error never accumulates. The time source, the sleep and
//! the thread boost are per platform: Windows QueryPerformanceCounter + a high-resolution waitable
//! timer + MMCSS (`win.rs`), macOS `mach_absolute_time` + `mach_wait_until` + a time-constraint
//! thread policy (`mac.rs`).

use crate::core::frame_out::FrameRate;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod win;

#[cfg(target_os = "macos")]
use mac as sys;
#[cfg(windows)]
use win as sys;

/// Microseconds on the system's monotonic clock (the same in every process on this machine).
pub use sys::now_us;
/// The calling thread scheduled ahead of ordinary work while the returned guard lives (output
/// clock and senders).
pub use sys::{boost_thread, Boost};

/// Ticks at a [`FrameRate`] from the moment it's created.
pub struct Ticker {
    timer: sys::Timer,
    start_us: i64,
    rate: FrameRate,
    index: i64,
}

/// One tick: its index, when it was due (µs, [`now_us`]), how late the wait returned (µs).
#[derive(Debug, Clone, Copy)]
pub struct Tick {
    pub index: i64,
    pub due_us: i64,
    pub late_us: i64,
}

impl Ticker {
    pub fn new(rate: FrameRate) -> Result<Self, String> {
        Ok(Self {
            timer: sys::Timer::new()?,
            start_us: now_us(),
            rate,
            index: 0,
        })
    }

    /// When tick `i` is due (µs, [`now_us`]).
    pub fn at(&self, i: i64) -> i64 {
        due_at(self.start_us, self.rate, i)
    }

    /// Sleeps until the next tick ([`next_tick`]): a tick already past returns at once; if the
    /// thread fell more than a frame behind, the ticks it missed are skipped (never a burst to
    /// catch up).
    pub fn wait(&mut self) -> Tick {
        let now = now_us();
        let (index, due) = next_tick(self.start_us, self.rate, self.index, now);
        self.index = index;
        if due > now {
            self.timer.sleep_until(due);
        }
        Tick {
            index,
            due_us: due,
            late_us: now_us() - due,
        }
    }
}

/// When tick `index` of a ticker started at `start_us` is due (µs). Computed from the index, so
/// rounding never accumulates.
fn due_at(start_us: i64, rate: FrameRate, index: i64) -> i64 {
    start_us + rate.tick_offset_ns(index) / 1000
}

/// The tick to wait for after tick `prev`, at time `now` (µs): normally `prev + 1`, also when it
/// is already a little past (the wait returns at once). When `now` is more than a frame past it,
/// the newest tick already due instead, skipping the ones in between. Returns (index, due).
fn next_tick(start_us: i64, rate: FrameRate, prev: i64, now: i64) -> (i64, i64) {
    let mut index = prev + 1;
    if now - due_at(start_us, rate, index) > rate.period_ns() / 1000 {
        while due_at(start_us, rate, index + 1) <= now {
            index += 1;
        }
    }
    (index, due_at(start_us, rate, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: i64 = 1_000_000;

    fn due(rate: FrameRate, i: i64) -> i64 {
        due_at(START, rate, i)
    }

    #[test]
    fn on_time_waits_for_the_next_tick() {
        let r = FrameRate::R60;
        assert_eq!(next_tick(START, r, 0, START), (1, due(r, 1)));
        assert_eq!(next_tick(START, r, 7, due(r, 8) - 5_000), (8, due(r, 8)));
    }

    #[test]
    fn slightly_late_still_takes_the_next_tick() {
        let r = FrameRate::R50;
        let period = r.period_ns() / 1000;
        // Late, but not by more than a frame: tick 4 now (no sleep), nothing skipped.
        assert_eq!(next_tick(START, r, 3, due(r, 4) + 1), (4, due(r, 4)));
        assert_eq!(next_tick(START, r, 3, due(r, 4) + period), (4, due(r, 4)));
    }

    #[test]
    fn more_than_a_frame_late_skips_to_the_newest_due_tick() {
        let r = FrameRate::R60;
        let period = r.period_ns() / 1000;
        // Just over a frame late: tick 5 is due as well, so 4 is skipped.
        assert_eq!(
            next_tick(START, r, 3, due(r, 4) + period + 1),
            (5, due(r, 5))
        );
        // 100 ms behind: the newest tick at or before now, never a burst.
        let now = due(r, 4) + 100_000;
        let (i, d) = next_tick(START, r, 3, now);
        assert!(d <= now && due(r, i + 1) > now, "tick {i}");
        assert_eq!(i, 10);
    }

    #[test]
    fn due_times_follow_the_index() {
        for r in [
            FrameRate::R25,
            FrameRate::R29_97,
            FrameRate::R59_94,
            FrameRate::R60,
        ] {
            let mut prev = 0;
            // A day of ticks, waited for on time: every tick, due exactly at its offset.
            let day = 24 * 3600 * 60;
            for i in (1..day).step_by(9_973) {
                let (index, d) = next_tick(START, r, i - 1, due(r, i) - 1);
                assert_eq!(index, i);
                assert_eq!(d, START + r.tick_offset_ns(i) / 1000);
                assert!(d >= prev);
                prev = d;
            }
        }
    }

    /// The platform timer works: waits come back, in order, not before their tick (loose
    /// bounds; the tick logic is tested above without a clock).
    #[test]
    fn the_platform_timer_wakes_for_ticks() {
        let t0 = now_us();
        let mut t = Ticker::new(FrameRate::R60).unwrap();
        let mut last = 0;
        for _ in 0..5 {
            let tick = t.wait();
            assert!(tick.index > last);
            assert!(tick.late_us >= -1_000, "woke {} µs early", -tick.late_us);
            last = tick.index;
        }
        let spent = now_us() - t0;
        assert!((50_000..2_000_000).contains(&spent), "{spent} µs");
    }

    #[test]
    fn now_is_monotonic_microseconds() {
        let a = now_us();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let b = now_us();
        assert!((15_000..200_000).contains(&(b - a)), "{} µs", b - a);
    }
}
