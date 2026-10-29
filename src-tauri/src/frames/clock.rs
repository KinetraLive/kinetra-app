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
        self.start_us + self.rate.tick_offset_ns(i) / 1000
    }

    /// Sleeps until the next tick. A tick already past returns at once; if the thread fell more
    /// than a frame behind, the ticks it missed are skipped (never a burst to catch up).
    pub fn wait(&mut self) -> Tick {
        self.index += 1;
        let now = now_us();
        let mut due = self.at(self.index);
        if now - due > self.rate.period_ns() / 1000 {
            // Jump to the newest tick that is already due.
            while self.at(self.index + 1) <= now {
                self.index += 1;
            }
            due = self.at(self.index);
        }
        if due > now {
            self.timer.sleep_until(due);
        }
        Tick {
            index: self.index,
            due_us: due,
            late_us: now_us() - due,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_follow_the_rate_and_skip_when_behind() {
        let mut t = Ticker::new(FrameRate::R60).unwrap();
        let first = t.wait();
        let second = t.wait();
        // Normally the next tick; a busy machine (CI) may wake more than a frame late, and the
        // ticker then skips ahead. Either way the due time follows from the index.
        let steps = second.index - first.index;
        assert!(steps >= 1);
        assert!((second.due_us - first.due_us - steps * 16_666).abs() <= steps);
        // Fall 100 ms behind: the next wait skips to the newest due tick instead of bursting.
        std::thread::sleep(std::time::Duration::from_millis(100));
        let after = t.wait();
        assert!(
            after.index >= second.index + 6,
            "skipped to tick {}",
            after.index
        );
        assert!(after.late_us < 17_000, "returned {} µs late", after.late_us);
    }

    #[test]
    fn now_is_monotonic_microseconds() {
        let a = now_us();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let b = now_us();
        assert!((15_000..200_000).contains(&(b - a)), "{} µs", b - a);
    }
}
