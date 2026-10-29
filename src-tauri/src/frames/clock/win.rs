//! Windows: QueryPerformanceCounter time, a high-resolution waitable timer (Windows 10 before 1803
//! has none: a plain waitable timer is used there, ~1 ms coarser wake-ups) and MMCSS.

use std::sync::OnceLock;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject,
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, TIMER_ALL_ACCESS,
};

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

/// The calling thread in the Multimedia Class Scheduler's "Playback" class while this lives
/// (output clock and senders: scheduled ahead of ordinary work without starving it); where MMCSS
/// isn't available, above-normal priority instead.
pub struct Boost(Option<HANDLE>);

pub fn boost_thread() -> Boost {
    use windows::core::w;
    use windows::Win32::System::Threading::{
        AvSetMmThreadCharacteristicsW, GetCurrentThread, SetThreadPriority,
        THREAD_PRIORITY_ABOVE_NORMAL,
    };
    let mut task = 0u32;
    match unsafe { AvSetMmThreadCharacteristicsW(w!("Playback"), &mut task) } {
        Ok(h) => Boost(Some(h)),
        Err(_) => {
            unsafe {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
            }
            Boost(None)
        }
    }
}

impl Drop for Boost {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            unsafe {
                let _ = windows::Win32::System::Threading::AvRevertMmThreadCharacteristics(h);
            }
        }
    }
}

/// The ticker's waitable timer.
pub struct Timer(HANDLE);

// SAFETY: the timer handle is only used by the thread that owns the ticker.
unsafe impl Send for Timer {}

impl Timer {
    pub fn new() -> Result<Self, String> {
        let h = unsafe {
            CreateWaitableTimerExW(
                None,
                None,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
            .or_else(|_| CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0))
            .map_err(|e| e.to_string())?
        };
        Ok(Self(h))
    }

    /// Sleeps until `due_us` ([`now_us`]).
    pub fn sleep_until(&mut self, due_us: i64) {
        let wait = due_us - now_us();
        if wait <= 0 {
            return;
        }
        // Relative time in 100 ns units (negative = relative).
        let rel = -(wait * 10);
        unsafe {
            let _ = SetWaitableTimer(self.0, &rel, 0, None, None, false);
            WaitForSingleObject(self.0, INFINITE);
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
