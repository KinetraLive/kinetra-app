//! A frame source: one captured page, converted at a fixed rate and handed to its senders.
//!
//! The output clock thread wakes on each tick ([`Ticker`]) and never waits on anything else:
//! - the page drew something new → convert it on the GPU ([`Converter::submit`]);
//! - collect the newest finished conversion ([`Converter::collect`]; the GPU had one tick);
//! - hand the newest frame to every outlet (a latest-wins `Mailbox`). Nothing new (still
//!   graphics, or the GPU not done) → the previous frame again, so receivers see a steady rate.
//!
//! A sender that doesn't take a frame before the next one replaces it, and the replaced one is
//! counted on that outlet as dropped. One slow sender never slows the clock or another sender.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::capture::{Capture, CaptureOptions, OnFailed, Page};
use super::clock::{boost_thread, Ticker};
use super::convert::{Collected, Converter};
use super::gpu::{self, Adapter, Gpu};
use super::outlet::Outlet;
use super::FrameError;
use crate::core::frame_out::{
    AlphaMode, Frame, FrameCounters, FrameCountersView, FrameRate, PixelFormat, Pool,
};

/// What a source captures and makes.
#[derive(Debug, Clone)]
pub struct SourceConfig {
    pub page: Page,
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    pub format: PixelFormat,
    pub alpha: AlphaMode,
    /// `Uyvy` frames are composited over this color.
    pub background: [u8; 3],
    /// The WebView2 user data folder for captures.
    pub data_dir: PathBuf,
    pub adapter: Adapter,
    /// Capture every frame the page draws (see `CaptureOptions::follow_page`); true in the app.
    pub follow_page: bool,
    /// The capture environment's browser arguments; `None` = the capture defaults.
    pub browser_args: Option<String>,
    /// The capture environment's language; `None` = WebView2's default.
    pub language: Option<String>,
}

pub struct FrameSource {
    pub config: SourceConfig,
    pub counters: Arc<FrameCounters>,
    /// The adapter the device is on.
    pub adapter_name: String,
    /// The WebView2 browser process of the capture.
    pub browser_pid: u32,
    outlets: Arc<Mutex<Vec<Arc<Outlet>>>>,
    stop: Arc<AtomicBool>,
    clock: Option<std::thread::JoinHandle<()>>,
    // Dropped after the clock thread has stopped (field order).
    capture: Option<Capture>,
    _gpu: Arc<Gpu>,
}

impl FrameSource {
    /// Starts capturing and ticking. `on_failed`: the page's process failed (see
    /// [`CaptureOptions::on_failed`]).
    pub fn start(config: SourceConfig, on_failed: Option<OnFailed>) -> Result<Self, FrameError> {
        let gpu = gpu::shared(config.adapter)?;
        let capture = Capture::start(
            &gpu,
            CaptureOptions {
                page: config.page.clone(),
                width: config.width,
                height: config.height,
                data_dir: config.data_dir.clone(),
                follow_page: config.follow_page,
                on_failed,
                browser_args: config.browser_args.clone(),
                language: config.language.clone(),
            },
        )?;
        let converter = Converter::new(
            &gpu.device,
            &capture.texture,
            config.width,
            config.height,
            config.format,
            config.alpha,
            config.background,
        )?;
        let counters = Arc::new(FrameCounters::default());
        let outlets: Arc<Mutex<Vec<Arc<Outlet>>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let ticker = Ticker::new(config.rate).map_err(|e| FrameError::Gpu(e.to_string()))?;
        let clock = {
            let job = ClockJob {
                gpu: gpu.clone(),
                converter,
                ticker,
                captured: capture.captured.clone(),
                counters: counters.clone(),
                outlets: outlets.clone(),
                stop: stop.clone(),
                pool: Pool::new(config.format.frame_bytes(config.width, config.height), 6),
                config: config.clone(),
            };
            std::thread::Builder::new()
                .name("frames-clock".into())
                .spawn(move || job.run())
                .map_err(|e| FrameError::Gpu(e.to_string()))?
        };
        Ok(Self {
            adapter_name: gpu.name.clone(),
            browser_pid: capture.browser_pid,
            config,
            counters,
            outlets,
            stop,
            clock: Some(clock),
            capture: Some(capture),
            _gpu: gpu,
        })
    }

    /// A new outlet: frames from the next tick on.
    pub fn subscribe(&self) -> Arc<Outlet> {
        let o = Arc::new(Outlet::default());
        if let Ok(mut v) = self.outlets.lock() {
            v.push(o.clone());
        }
        o
    }

    /// Stops feeding `outlet` and closes it.
    pub fn unsubscribe(&self, outlet: &Arc<Outlet>) {
        if let Ok(mut v) = self.outlets.lock() {
            v.retain(|o| !Arc::ptr_eq(o, outlet));
        }
        outlet.mailbox.close();
    }

    pub fn counters(&self) -> FrameCountersView {
        self.counters.view()
    }
}

impl Drop for FrameSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.clock.take() {
            t.join().ok();
        }
        if let Ok(v) = self.outlets.lock() {
            for o in v.iter() {
                o.mailbox.close();
            }
        }
        self.capture.take();
    }
}

struct ClockJob {
    gpu: Arc<Gpu>,
    converter: Converter,
    ticker: Ticker,
    captured: Arc<AtomicU64>,
    counters: Arc<FrameCounters>,
    outlets: Arc<Mutex<Vec<Arc<Outlet>>>>,
    stop: Arc<AtomicBool>,
    pool: Arc<Pool>,
    config: SourceConfig,
}

impl ClockJob {
    fn run(mut self) {
        let _boost = boost_thread();
        let period_us = self.config.rate.period_ns() / 1000;
        let mut seen = 0u64;
        let mut last: Option<Arc<Frame>> = None;
        let mut submitted_at: Option<i64> = None;
        // Conversions submitted before the source went idle are stale when it wakes.
        let mut stale_upto = 0u64;
        let mut idle = false;
        while !self.stop.load(Ordering::Relaxed) {
            let tick = self.ticker.wait();
            let c = &self.counters;
            c.ticks.fetch_add(1, Ordering::Relaxed);
            if tick.late_us > period_us {
                c.missed.fetch_add(1, Ordering::Relaxed);
            }
            let now_captured = self.captured.load(Ordering::Relaxed);
            c.captured.store(now_captured, Ordering::Relaxed);
            let wanted = self
                .outlets
                .lock()
                .map(|o| o.iter().any(|o| o.wanted.load(Ordering::Relaxed)))
                .unwrap_or(true);
            if !wanted {
                // Nobody receives: no GPU work, nothing handed out (a stale frame never reaches
                // the next receiver).
                if !idle {
                    idle = true;
                    stale_upto = self.converter.last_seq();
                    self.converter.release_staging();
                    self.pool.trim();
                    last = None;
                    submitted_at = None;
                }
                c.idle.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if idle {
                // Awake: convert what the page shows now, even if it drew nothing new.
                idle = false;
                seen = u64::MAX;
            }
            let fresh = now_captured != seen;
            if fresh && self.converter.can_submit() {
                seen = now_captured;
                if self.converter.submit(&self.gpu.context).is_some() && submitted_at.is_none() {
                    submitted_at = Some(tick.index);
                }
            }
            let mut buf = self.pool.take();
            let frame = match self.converter.collect(&self.gpu.context, &mut buf) {
                Ok(Collected::Frame(seq)) if seq <= stale_upto => None,
                Ok(Collected::Frame(_)) => {
                    submitted_at = None;
                    Some(Arc::new(Frame {
                        data: buf,
                        width: self.config.width,
                        height: self.config.height,
                        format: self.config.format,
                        alpha: self.config.alpha,
                        rate: self.config.rate,
                        tick: tick.index,
                        due_us: tick.due_us,
                    }))
                }
                Ok(Collected::Pending) => {
                    // The GPU had a whole tick and isn't done: late (a pipeline tick is normal).
                    if submitted_at.is_some_and(|t| t < tick.index) {
                        c.late.fetch_add(1, Ordering::Relaxed);
                    }
                    None
                }
                Ok(Collected::Idle) => {
                    if last.is_some() {
                        c.repeated.fetch_add(1, Ordering::Relaxed);
                    }
                    None
                }
                Err(e) => {
                    tracing::warn!("frames: read-back failed: {e}");
                    None
                }
            };
            if let Some(f) = frame {
                last = Some(f);
            }
            if let Some(f) = &last {
                if let Ok(outlets) = self.outlets.lock() {
                    for o in outlets.iter() {
                        if o.mailbox.put(f.clone()).is_some() {
                            o.dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        }
    }
}
