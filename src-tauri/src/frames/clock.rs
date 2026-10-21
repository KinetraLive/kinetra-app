//! The output clock: QueryPerformanceCounter time and a ticker on a high-resolution waitable
//! timer with absolute deadlines from the tick index (`FrameRate::tick_offset_ns`), so the error
//! never accumulates. Windows 10 before 1803 has no high-resolution timers: a plain waitable timer
//! is used there (~1 ms coarser wake-ups).

use std::sync::OnceLock;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject,
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, TIMER_ALL_ACCESS,
};

use crate::core::frame_out::FrameRate;

fn qpc_frequency() -> i64 {
    static F: OnceLock<i64> = OnceLock::new();
    *F.get_or_init(|| {
        let mut f = 0i64;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut f);
        }
        f.max(1)
    })
}

/// Microseconds on the QPC clock (the same in every process on this machine).
pub fn now_us() -> i64 {
    let mut c = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
    }
    (i128::from(c) * 1_000_000 / i128::from(qpc_frequency())) as i64
}

/// Ticks at a [`FrameRate`] from the moment it's created.
pub struct Ticker {
    timer: HANDLE,
    start_us: i64,
    rate: FrameRate,
    index: i64,
}

// SAFETY: the timer handle is only used by the thread that owns the ticker.
unsafe impl Send for Ticker {}

/// One tick: its index, when it was due (µs, QPC), how late the wait returned (µs).
#[derive(Debug, Clone, Copy)]
pub struct Tick {
    pub index: i64,
    pub due_us: i64,
    pub late_us: i64,
}

impl Ticker {
    pub fn new(rate: FrameRate) -> windows::core::Result<Self> {
        let timer = unsafe {
            CreateWaitableTimerExW(
                None,
                None,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
            .or_else(|_| CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0))?
        };
        Ok(Self {
            timer,
            start_us: now_us(),
            rate,
            index: 0,
        })
    }

    /// When tick `i` is due (µs, QPC).
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
        let wait = due - now;
        if wait > 0 {
            // Relative time in 100 ns units (negative = relative).
            let rel = -(wait * 10);
            unsafe {
                let _ = SetWaitableTimer(self.timer, &rel, 0, None, None, false);
                WaitForSingleObject(self.timer, INFINITE);
            }
        }
        Tick {
            index: self.index,
            due_us: due,
            late_us: now_us() - due,
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.timer);
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
        assert_eq!(second.index, first.index + 1);
        assert!((second.due_us - first.due_us - 16_666).abs() <= 1);
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
}
