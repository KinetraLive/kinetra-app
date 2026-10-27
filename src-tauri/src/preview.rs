//! The remote preview's capture host (Windows; `core::remote::preview`, protocol 2 §6).
//!
//! One page capture per demanded stream, rendered at the picture's size (the renderer's output
//! mode fits the screen and paints the checkerboard behind transparency, so the capture is
//! opaque and needs no GPU pass of its own), and ONE worker thread for all of them, at
//! background priority, created with the first capture and gone with the last:
//!
//! - at each stream's slot (the sink's interval: the demand's rate, 1 per second on air) it
//!   copies the newest captured frame into a small staging texture if the page drew since;
//! - a moment later it maps it **without waiting** (`DO_NOT_WAIT`: the device's lock is shared
//!   with the NDI clocks, so this thread never blocks inside it), hashes the pixels and, only
//!   when they changed, encodes a JPEG (quality stepping down to fit the server's limit) into the
//!   sink;
//! - it samples the CPU every second and reports late pictures for the busy rule; while the sink
//!   is paused (busy) it does no capture work at all.
//!
//! Nothing here is logged per picture or stored.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_BIND_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_FLAG_DO_NOT_WAIT,
    D3D11_MAP_READ,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAS_STILL_DRAWING;
use windows::Win32::System::Threading::GetSystemTimes;

use crate::core::cloud::projects::scan::BackgroundPriority;
use crate::core::fsx::now_epoch_ms;
use crate::core::remote::preview::{
    CapturePlan, PreviewFrame, PreviewHost, PreviewPlan, PreviewSink, HEADER_BYTES,
};
use crate::frames::capture::{Capture, CaptureOptions};
use crate::frames::gpu::{self, texture, Adapter, Gpu};
use crate::frames::jpeg::Jpeg;
use crate::frames::Page;
use crate::ndi::host::AppEnvironment;

/// Above this share of busy CPU in a sample, the busy rule counts the second.
const CPU_HIGH: f64 = 0.85;
const CPU_EVERY: Duration = Duration::from_secs(1);
/// How soon a copied frame is mapped again while the GPU is still copying.
const MAP_RETRY: Duration = Duration::from_millis(4);
/// The lowest JPEG quality the step-down goes to.
const QUALITY_MIN: f32 = 0.2;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One running capture.
struct Cap {
    plan: CapturePlan,
    staging: ID3D11Texture2D,
    // Dropped last (field order): its UI thread and page.
    capture: Capture,
}

// SAFETY: D3D11 objects are free-threaded; every context call goes through the device's
// multithread-protected immediate context. A `Cap` is read by the worker only.
unsafe impl Send for Cap {}
unsafe impl Sync for Cap {}

#[derive(Default)]
struct Shared {
    caps: Vec<Arc<Cap>>,
    sink: Option<Arc<PreviewSink>>,
    quality: f32,
    max_jpeg: usize,
    stop: bool,
}

struct Worker {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        lock(&self.shared.0).stop = true;
        self.shared.1.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub struct PreviewCaptures {
    /// The capture pages' own WebView2 folder (NDI's: the same options share its browser).
    data_dir: PathBuf,
    /// Share the app's browser process when possible.
    app_env: Option<AppEnvironment>,
    own_env: AtomicBool,
    /// Held for a whole `apply`.
    run: Mutex<Running>,
}

#[derive(Default)]
struct Running {
    caps: HashMap<u16, Arc<Cap>>,
    worker: Option<Worker>,
    gpu: Option<Arc<Gpu>>,
}

impl PreviewCaptures {
    pub fn new(data_dir: PathBuf, app_env: Option<AppEnvironment>) -> Arc<Self> {
        Arc::new(Self {
            data_dir,
            app_env,
            own_env: AtomicBool::new(false),
            run: Mutex::default(),
        })
    }

    fn start(&self, gpu: &Gpu, plan: &CapturePlan) -> Result<Cap, String> {
        let opts = |env: Option<&AppEnvironment>| CaptureOptions {
            page: Page::Url(plan.url.clone()),
            width: plan.width,
            height: plan.height,
            data_dir: env.map_or_else(|| self.data_dir.clone(), |e| e.data_dir.clone()),
            follow_page: false,
            on_failed: None,
            browser_args: env.map(|e| e.browser_args.clone()),
            language: env.map(|e| e.language.clone()),
        };
        let shared = self
            .app_env
            .as_ref()
            .filter(|_| !self.own_env.load(Ordering::Relaxed));
        let capture = match shared {
            Some(env) => match Capture::start(gpu, opts(Some(env))) {
                Ok(c) => Ok(c),
                Err(e) => {
                    tracing::warn!("preview capture can't share the app's WebView2 ({e})");
                    self.own_env.store(true, Ordering::Relaxed);
                    Capture::start(gpu, opts(None))
                }
            },
            None => Capture::start(gpu, opts(None)),
        }
        .map_err(|e| e.to_string())?;
        let staging = texture(
            &gpu.device,
            plan.width,
            plan.height,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            D3D11_BIND_FLAG(0),
            true,
        )
        .map_err(|e| e.to_string())?;
        Ok(Cap {
            plan: plan.clone(),
            staging,
            capture,
        })
    }
}

impl PreviewHost for PreviewCaptures {
    fn apply(&self, plan: &PreviewPlan, sink: &Arc<PreviewSink>) -> HashMap<u16, String> {
        let mut run = lock(&self.run);
        let mut failed = HashMap::new();
        // Stop what is gone or changed.
        run.caps
            .retain(|s, c| plan.captures.iter().any(|p| p.stream == *s && *p == c.plan));
        if plan.captures.is_empty() {
            // Nothing watched: no worker, no pages, no device.
            run.caps.clear();
            run.worker = None;
            run.gpu = None;
            return failed;
        }
        let gpu = match run.gpu.clone() {
            Some(g) => g,
            None => match gpu::shared(Adapter::Primary) {
                Ok(g) => {
                    run.gpu = Some(g.clone());
                    g
                }
                Err(e) => {
                    for p in &plan.captures {
                        failed.insert(p.stream, e.to_string());
                    }
                    return failed;
                }
            },
        };
        for p in &plan.captures {
            if run.caps.contains_key(&p.stream) {
                continue;
            }
            match self.start(&gpu, p) {
                Ok(c) => {
                    run.caps.insert(p.stream, Arc::new(c));
                }
                Err(e) => {
                    tracing::warn!("preview capture of stream {} failed: {e}", p.stream);
                    failed.insert(p.stream, e);
                }
            }
        }
        let caps: Vec<Arc<Cap>> = run.caps.values().cloned().collect();
        let quality = f32::from(plan.quality.clamp(1, 100)) / 100.0;
        let max_jpeg = plan.max_bytes.saturating_sub(HEADER_BYTES);
        match &run.worker {
            Some(w) => {
                let mut g = lock(&w.shared.0);
                g.caps = caps;
                g.sink = Some(sink.clone());
                g.quality = quality;
                g.max_jpeg = max_jpeg;
                drop(g);
                w.shared.1.notify_all();
            }
            None if !caps.is_empty() => {
                let shared = Arc::new((
                    Mutex::new(Shared {
                        caps,
                        sink: Some(sink.clone()),
                        quality,
                        max_jpeg,
                        stop: false,
                    }),
                    Condvar::new(),
                ));
                let job = shared.clone();
                let g = gpu.clone();
                match std::thread::Builder::new()
                    .name("remote-preview".into())
                    .spawn(move || work(&job, &g))
                {
                    Ok(t) => {
                        run.worker = Some(Worker {
                            shared,
                            thread: Some(t),
                        })
                    }
                    Err(e) => {
                        for p in &plan.captures {
                            failed.insert(p.stream, e.to_string());
                        }
                    }
                }
            }
            None => {}
        }
        failed
    }
}

/// Per stream, in the worker.
struct Slot {
    due: Instant,
    /// The frame count of the capture when last copied.
    seen: u64,
    /// A copy is on the GPU since (the slot it was for).
    copying: Option<Instant>,
    hash: Option<u64>,
}

/// FNV-1a over the picture's rows (64-bit words), padding excluded.
fn hash(rows: &[u8], row_bytes: usize, pitch: usize, height: usize) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for y in 0..height {
        let row = &rows[y * pitch..y * pitch + row_bytes];
        let (words, rest) = row.as_chunks::<8>();
        for c in words {
            h = (h ^ u64::from_le_bytes(*c)).wrapping_mul(0x0100_0000_01b3);
        }
        for &b in rest {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

fn filetime(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

/// The share of busy CPU since the last call.
struct Cpu {
    last: Option<(u64, u64)>,
}

impl Cpu {
    fn sample(&mut self) -> Option<f64> {
        let (mut idle, mut kernel, mut user) = Default::default();
        // SAFETY: three out-pointers to FILETIMEs on the stack.
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
        // Kernel time includes idle time.
        let (idle, total) = (filetime(idle), filetime(kernel) + filetime(user));
        let prev = self.last.replace((idle, total));
        let (pi, pt) = prev?;
        let dt = total.saturating_sub(pt);
        (dt > 0).then(|| 1.0 - idle.saturating_sub(pi) as f64 / dt as f64)
    }
}

fn work(shared: &Arc<(Mutex<Shared>, Condvar)>, gpu: &Gpu) {
    let _bg = BackgroundPriority::enter();
    let mut jpeg: Option<(usize, Jpeg)> = None;
    let mut slots: HashMap<u16, Slot> = HashMap::new();
    let mut bgr: Vec<u8> = Vec::new();
    let mut cpu = Cpu { last: None };
    let mut cpu_at = Instant::now();
    let ctx = &gpu.context;
    loop {
        let (caps, sink, quality, max_jpeg) = {
            let g = lock(&shared.0);
            if g.stop {
                return;
            }
            (g.caps.clone(), g.sink.clone(), g.quality, g.max_jpeg)
        };
        let Some(sink) = sink else { return };
        let now = Instant::now();
        if now >= cpu_at {
            cpu_at = now + CPU_EVERY;
            if let Some(share) = cpu.sample() {
                sink.cpu(share > CPU_HIGH);
            }
        }
        slots.retain(|s, _| caps.iter().any(|c| c.plan.stream == *s));
        let interval = sink.interval();
        let mut wake = now + CPU_EVERY;
        if !sink.paused() {
            for c in &caps {
                let slot = slots.entry(c.plan.stream).or_insert(Slot {
                    due: now,
                    seen: u64::MAX,
                    copying: None,
                    hash: None,
                });
                if let Some(due) = slot.copying {
                    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
                    let mapped = unsafe {
                        ctx.Map(
                            &c.staging,
                            0,
                            D3D11_MAP_READ,
                            D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                            Some(&mut m),
                        )
                    };
                    match mapped {
                        Err(e) if e.code() == DXGI_ERROR_WAS_STILL_DRAWING => {
                            wake = wake.min(Instant::now() + MAP_RETRY);
                            continue;
                        }
                        Err(e) => {
                            tracing::warn!("preview: read-back failed: {e}");
                            slot.copying = None;
                            wake = wake.min(slot.due);
                            continue;
                        }
                        Ok(()) => {}
                    }
                    slot.copying = None;
                    let (w, h) = (c.plan.width as usize, c.plan.height as usize);
                    let pitch = m.RowPitch as usize;
                    if m.pData.is_null() || pitch < w * 4 {
                        unsafe { ctx.Unmap(&c.staging, 0) };
                        wake = wake.min(slot.due);
                        continue;
                    }
                    // SAFETY: the mapped staging texture is h rows of `pitch` bytes.
                    let data =
                        unsafe { std::slice::from_raw_parts(m.pData as *const u8, pitch * h) };
                    let hv = hash(data, w * 4, pitch, h);
                    let changed = slot.hash != Some(hv);
                    if changed {
                        bgr.resize(w * h * 3, 0);
                        for y in 0..h {
                            let src = &data[y * pitch..y * pitch + w * 4];
                            let dst = &mut bgr[y * w * 3..(y + 1) * w * 3];
                            let (bgr3, _) = dst.as_chunks_mut::<3>();
                            for (d, s) in bgr3.iter_mut().zip(src.as_chunks::<4>().0) {
                                *d = [s[0], s[1], s[2]];
                            }
                        }
                    }
                    unsafe { ctx.Unmap(&c.staging, 0) };
                    if changed {
                        slot.hash = Some(hv);
                        if jpeg.as_ref().is_none_or(|(m, _)| *m != max_jpeg) {
                            jpeg = Jpeg::new(max_jpeg).ok().map(|j| (max_jpeg, j));
                        }
                        if let Some((_, j)) = jpeg.as_mut() {
                            let mut q = quality;
                            loop {
                                match j.encode(w as u32, h as u32, (w * 3) as u32, &bgr, q) {
                                    Ok(Some(out)) => {
                                        sink.put(
                                            c.plan.stream,
                                            PreviewFrame {
                                                jpeg: out.to_vec(),
                                                width: w as u16,
                                                height: h as u16,
                                                captured_ms: now_epoch_ms(),
                                            },
                                        );
                                        break;
                                    }
                                    Ok(None) if q > QUALITY_MIN => q = (q - 0.15).max(QUALITY_MIN),
                                    Ok(None) => break,
                                    Err(e) => {
                                        tracing::warn!("preview: {e}");
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    // Late: the picture is ready after its next slot should have started.
                    sink.on_time(Instant::now() <= due + interval);
                    wake = wake.min(slot.due);
                    continue;
                }
                if now >= slot.due {
                    // The next slot: from the schedule, or from now when far behind.
                    let due = slot.due;
                    slot.due = if now > due + interval {
                        now + interval
                    } else {
                        due + interval
                    };
                    let drawn = c.capture.captured.load(Ordering::Relaxed);
                    if drawn != slot.seen {
                        slot.seen = drawn;
                        unsafe {
                            ctx.CopyResource(&c.staging, &c.capture.texture);
                            ctx.Flush();
                        }
                        slot.copying = Some(due);
                        wake = wake.min(Instant::now() + MAP_RETRY);
                        continue;
                    }
                }
                wake = wake.min(slot.due);
            }
        }
        let g = lock(&shared.0);
        if g.stop {
            return;
        }
        let wait = wake.saturating_duration_since(Instant::now());
        if !wait.is_zero() {
            let _ = shared.1.wait_timeout(g, wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_ignores_row_padding_and_sees_changes() {
        let (w, h, pitch) = (3usize, 2usize, 16usize);
        let mut a = vec![0u8; pitch * h];
        let mut b = vec![0u8; pitch * h];
        for y in 0..h {
            for x in 0..w * 4 {
                a[y * pitch + x] = (y * 31 + x) as u8;
                b[y * pitch + x] = (y * 31 + x) as u8;
            }
        }
        b[w * 4 + 2] = 0xee; // padding of row 0
        assert_eq!(hash(&a, w * 4, pitch, h), hash(&b, w * 4, pitch, h));
        b[pitch + 1] ^= 1;
        assert_ne!(hash(&a, w * 4, pitch, h), hash(&b, w * 4, pitch, h));
    }

    #[test]
    fn the_cpu_share_is_between_0_and_1() {
        let mut c = Cpu { last: None };
        assert!(c.sample().is_none());
        let mut x = 0u64;
        for i in 0..5_000_000u64 {
            x = x.wrapping_add(i * i);
        }
        std::hint::black_box(x);
        if let Some(s) = c.sample() {
            assert!((0.0..=1.0).contains(&s), "{s}");
        }
    }
}
