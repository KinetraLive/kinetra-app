//! Frame outputs (NDI now, OMT later): the parts that don't touch the GPU or a library, shared by
//! the frame source (`crate::frames`) and its senders (`crate::ndi`).
//!
//! - [`FrameRate`]: the six output rates as exact fractions, and tick times without drift.
//! - [`PixelFormat`] / [`AlphaMode`]: what a frame holds and how its alpha is meant.
//! - [`Pool`]: reused frame buffers (one 1080p UYVA frame is 6.2 MB; allocating one per tick
//!   would churn the heap at 50–60 Hz).
//! - [`Mailbox`]: a one-slot, latest-wins hand-over between the output clock and a sender. The
//!   clock never waits on a sender; a frame the sender hasn't taken yet is replaced and counted
//!   as dropped.
//! - [`FrameCounters`] / [`Samples`]: what happened, for status and measurements.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

/// An output frame rate, `num / den` frames per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub const R25: Self = Self { num: 25, den: 1 };
    pub const R29_97: Self = Self {
        num: 30000,
        den: 1001,
    };
    pub const R30: Self = Self { num: 30, den: 1 };
    pub const R50: Self = Self { num: 50, den: 1 };
    pub const R59_94: Self = Self {
        num: 60000,
        den: 1001,
    };
    pub const R60: Self = Self { num: 60, den: 1 };
    /// Every rate an output may use, slowest first.
    pub const ALL: [Self; 6] = [
        Self::R25,
        Self::R29_97,
        Self::R30,
        Self::R50,
        Self::R59_94,
        Self::R60,
    ];

    pub fn fps(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    /// The time of tick `index` after the first one, in ns. Computed from the index every time,
    /// so rounding never accumulates (29.97 stays exact over a day).
    pub fn tick_offset_ns(self, index: i64) -> i64 {
        (i128::from(index) * 1_000_000_000 * i128::from(self.den) / i128::from(self.num)) as i64
    }

    /// One frame's duration in ns (rounded down).
    pub fn period_ns(self) -> i64 {
        self.tick_offset_ns(1)
    }

    /// `"25"`, `"29.97"`, …, `"60"`: how the UI and logs write it.
    pub fn label(self) -> String {
        if self.den == 1 {
            self.num.to_string()
        } else {
            format!("{:.2}", self.fps())
        }
    }

    /// Parses a [`label`](Self::label) (also `"59.94"`-style input); only the six rates.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.label() == s.trim())
    }
}

/// What a frame's bytes are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 4:2:2 Y'CbCr (BT.709, limited range) in UYVY rows, then an 8-bit alpha plane: fill and
    /// key in one frame.
    Uyva,
    /// 4:2:2 Y'CbCr without alpha (composited over a background color).
    Uyvy,
    /// 8-bit BGRA (the fallback: the receiver side converts on the CPU).
    Bgra,
}

impl PixelFormat {
    /// Bytes per row of the first (or only) plane.
    pub fn stride(self, width: u32) -> usize {
        match self {
            Self::Uyva | Self::Uyvy => width as usize * 2,
            Self::Bgra => width as usize * 4,
        }
    }

    /// Bytes of one whole frame.
    pub fn frame_bytes(self, width: u32, height: u32) -> usize {
        let plane = self.stride(width) * height as usize;
        match self {
            Self::Uyva => plane + width as usize * height as usize,
            Self::Uyvy | Self::Bgra => plane,
        }
    }
}

/// How the color of a frame with alpha relates to its alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlphaMode {
    /// Color as if opaque; alpha separate (what most receivers assume).
    #[default]
    Straight,
    /// Color already multiplied by alpha (what the browser composites).
    Premultiplied,
}

/// Reused byte buffers of one size. Buffers come back when a [`PooledBuf`] drops; at most `keep`
/// are kept, more are freed.
pub struct Pool {
    size: usize,
    keep: usize,
    free: Mutex<Vec<Vec<u8>>>,
}

impl Pool {
    pub fn new(size: usize, keep: usize) -> Arc<Self> {
        Arc::new(Self {
            size,
            keep,
            free: Mutex::new(Vec::new()),
        })
    }

    /// A buffer of `size` bytes (contents undefined: the caller overwrites all of it).
    pub fn take(self: &Arc<Self>) -> PooledBuf {
        let reused = self.free.lock().ok().and_then(|mut f| f.pop());
        PooledBuf {
            buf: reused.unwrap_or_else(|| vec![0; self.size]),
            pool: Arc::downgrade(self),
        }
    }

    /// Frees the buffers waiting to be reused (the source went idle).
    pub fn trim(&self) {
        if let Ok(mut f) = self.free.lock() {
            *f = Vec::new();
        }
    }

    /// Buffers waiting to be reused (tests).
    pub fn idle(&self) -> usize {
        self.free.lock().map(|f| f.len()).unwrap_or(0)
    }
}

/// A buffer from a [`Pool`]; goes back to it on drop.
pub struct PooledBuf {
    buf: Vec<u8>,
    pool: Weak<Pool>,
}

impl std::ops::Deref for PooledBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.buf
    }
}

impl std::ops::DerefMut for PooledBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

impl Drop for PooledBuf {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            if let Ok(mut free) = pool.free.lock() {
                if free.len() < pool.keep && self.buf.len() == pool.size {
                    free.push(std::mem::take(&mut self.buf));
                }
            }
        }
    }
}

/// One output frame, shared by every sender of a source (`Arc<Frame>`).
pub struct Frame {
    pub data: PooledBuf,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub alpha: AlphaMode,
    pub rate: FrameRate,
    /// The output tick it was made for (0 = the first).
    pub tick: i64,
    /// When that tick was due, µs on the QPC clock.
    pub due_us: i64,
}

impl Frame {
    pub fn stride(&self) -> usize {
        self.format.stride(self.width)
    }
}

/// A one-slot, latest-wins hand-over. `put` never blocks; `take` waits up to a timeout.
pub struct Mailbox<T> {
    state: Mutex<(Option<T>, bool)>,
    ready: Condvar,
}

impl<T> Default for Mailbox<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new((None, false)),
            ready: Condvar::new(),
        }
    }
}

impl<T> Mailbox<T> {
    /// Leaves `item` for the taker. Returns the item it replaced (not taken in time), or the item
    /// itself when the mailbox is closed.
    pub fn put(&self, item: T) -> Option<T> {
        let Ok(mut s) = self.state.lock() else {
            return Some(item);
        };
        if s.1 {
            return Some(item);
        }
        let old = s.0.replace(item);
        drop(s);
        self.ready.notify_one();
        old
    }

    /// The waiting item, waiting up to `timeout` for one. `None` on timeout or once closed and
    /// empty.
    pub fn take(&self, timeout: Duration) -> Option<T> {
        let deadline = Instant::now() + timeout;
        let mut s = self.state.lock().ok()?;
        loop {
            if let Some(item) = s.0.take() {
                return Some(item);
            }
            if s.1 {
                return None;
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            s = self.ready.wait_timeout(s, deadline - now).ok()?.0;
        }
    }

    /// Stops the hand-over: later `put`s are refused and a waiting `take` returns.
    pub fn close(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.1 = true;
            s.0 = None;
        }
        self.ready.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.state.lock().map(|s| s.1).unwrap_or(true)
    }
}

/// Counters of one frame source (all monotonic; readers diff them).
#[derive(Default)]
pub struct FrameCounters {
    /// Output ticks handled.
    pub ticks: AtomicU64,
    /// Frames the page produced (captures).
    pub captured: AtomicU64,
    /// Ticks that reused the previous frame because the page drew nothing new (normal for still
    /// graphics).
    pub repeated: AtomicU64,
    /// Ticks that reused the previous frame because the GPU hadn't finished the new one.
    pub late: AtomicU64,
    /// Ticks whose clock woke more than one frame late (the machine was busy).
    pub missed: AtomicU64,
    /// Ticks skipped because no sender wanted frames (no receiver).
    pub idle: AtomicU64,
}

/// A snapshot of [`FrameCounters`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameCountersView {
    pub ticks: u64,
    pub captured: u64,
    pub repeated: u64,
    pub late: u64,
    pub missed: u64,
    pub idle: u64,
}

impl FrameCounters {
    pub fn view(&self) -> FrameCountersView {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        FrameCountersView {
            ticks: g(&self.ticks),
            captured: g(&self.captured),
            repeated: g(&self.repeated),
            late: g(&self.late),
            missed: g(&self.missed),
            idle: g(&self.idle),
        }
    }
}

/// Timing samples in µs for measurements; off (nothing kept) unless enabled. Capped so a
/// forgotten switch can't grow without bound (one hour at 60 Hz).
#[derive(Default)]
pub struct Samples {
    on: std::sync::atomic::AtomicBool,
    values: Mutex<Vec<u32>>,
}

const SAMPLES_MAX: usize = 216_000;

impl Samples {
    pub fn enable(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
    }

    pub fn push(&self, d: Duration) {
        if self.on.load(Ordering::Relaxed) {
            if let Ok(mut v) = self.values.lock() {
                if v.len() < SAMPLES_MAX {
                    v.push(d.as_micros().min(u128::from(u32::MAX)) as u32);
                }
            }
        }
    }

    /// Takes the samples so far (sorted ascending), leaving none.
    pub fn drain_sorted(&self) -> Vec<u32> {
        let mut v = self
            .values
            .lock()
            .map(|mut v| std::mem::take(&mut *v))
            .unwrap_or_default();
        v.sort_unstable();
        v
    }
}

/// Nearest-rank percentile of sorted values (`p` in 0..=1); `None` when empty.
pub fn percentile(sorted: &[u32], p: f64) -> Option<u32> {
    if sorted.is_empty() {
        return None;
    }
    let i = ((sorted.len() - 1) as f64 * p.clamp(0.0, 1.0)).round() as usize;
    Some(sorted[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_are_exact_fractions_without_drift() {
        assert_eq!(FrameRate::R50.period_ns(), 20_000_000);
        assert_eq!(FrameRate::R25.tick_offset_ns(25), 1_000_000_000);
        // 29.97 over 24 h: tick 2_589_408 (= 24 h × 30000/1001) lands on the day exactly.
        let day_ticks = 24 * 3600 * 30000 / 1001;
        let t = FrameRate::R29_97.tick_offset_ns(day_ticks);
        assert!((t - 86_400_000_000_000).abs() < FrameRate::R29_97.period_ns());
        // No accumulation: summing periods drifts, the index formula doesn't.
        let summed = FrameRate::R59_94.period_ns() * 1_000_000;
        assert_ne!(summed, FrameRate::R59_94.tick_offset_ns(1_000_000));
        assert_eq!(FrameRate::R59_94.tick_offset_ns(60_000), 1_001_000_000_000);
    }

    #[test]
    fn rate_labels_round_trip() {
        let labels: Vec<String> = FrameRate::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(labels, ["25", "29.97", "30", "50", "59.94", "60"]);
        for r in FrameRate::ALL {
            assert_eq!(FrameRate::parse(&r.label()), Some(r));
        }
        assert_eq!(FrameRate::parse("24"), None);
        assert_eq!(FrameRate::parse(" 50 "), Some(FrameRate::R50));
    }

    #[test]
    fn frame_sizes() {
        assert_eq!(PixelFormat::Uyva.frame_bytes(1920, 1080), 1920 * 1080 * 3);
        assert_eq!(PixelFormat::Uyvy.frame_bytes(1920, 1080), 1920 * 1080 * 2);
        assert_eq!(PixelFormat::Bgra.frame_bytes(1920, 1080), 1920 * 1080 * 4);
        assert_eq!(PixelFormat::Uyva.stride(1280), 2560);
    }

    #[test]
    fn pool_reuses_up_to_keep() {
        let pool = Pool::new(16, 2);
        let (a, b, c) = (pool.take(), pool.take(), pool.take());
        assert_eq!(a.len(), 16);
        drop((a, b, c));
        assert_eq!(pool.idle(), 2);
        let d = pool.take();
        assert_eq!(pool.idle(), 1);
        pool.trim();
        assert_eq!(pool.idle(), 0);
        drop(pool);
        drop(d); // the pool is gone: the buffer is just freed
    }

    #[test]
    fn mailbox_keeps_only_the_latest() {
        let m = Mailbox::default();
        assert_eq!(m.put(1), None);
        assert_eq!(
            m.put(2),
            Some(1),
            "an untaken item is replaced and handed back"
        );
        assert_eq!(m.take(Duration::ZERO), Some(2));
        assert_eq!(m.take(Duration::from_millis(5)), None);
    }

    #[test]
    fn mailbox_close_wakes_the_taker_and_refuses_puts() {
        let m = Arc::new(Mailbox::default());
        let m2 = m.clone();
        let t = std::thread::spawn(move || m2.take(Duration::from_secs(10)));
        std::thread::sleep(Duration::from_millis(20));
        m.close();
        assert_eq!(t.join().ok().flatten(), None::<i32>);
        assert_eq!(m.put(3), Some(3));
        assert!(m.is_closed());
    }

    #[test]
    fn mailbox_take_wakes_on_put() {
        let m = Arc::new(Mailbox::default());
        let m2 = m.clone();
        let t = std::thread::spawn(move || m2.take(Duration::from_secs(10)));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(m.put(7), None);
        assert_eq!(t.join().ok().flatten(), Some(7));
    }

    #[test]
    fn samples_only_when_enabled() {
        let s = Samples::default();
        s.push(Duration::from_millis(1));
        assert!(s.drain_sorted().is_empty());
        s.enable(true);
        for ms in [3, 1, 2] {
            s.push(Duration::from_millis(ms));
        }
        let v = s.drain_sorted();
        assert_eq!(v, [1000, 2000, 3000]);
        assert_eq!(percentile(&v, 0.5), Some(2000));
        assert_eq!(percentile(&v, 0.99), Some(3000));
        assert_eq!(percentile(&[], 0.5), None);
    }
}
