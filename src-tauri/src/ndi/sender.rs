//! One NDI® source: a thread that takes the newest frame from its outlet and sends it.
//!
//! The thread is the only place NDI is called for the source, so NDI's encoding (SpeedHQ, on the
//! CPU) and network back-pressure stay here: the output clock hands frames over through a
//! latest-wins mailbox and never waits. Once a second it asks NDI for the receiver count and
//! tally; while no receiver is connected it asks on every frame instead and sends nothing.
//!
//! Two send modes, both measured by `examples/ndi_probe.rs`:
//! - [`SendMode::Sync`]: `NDIlib_send_send_video_v2` returns once the frame is encoded and
//!   queued; the time in the call is the encode cost.
//! - [`SendMode::Async`]: `NDIlib_send_send_video_async_v2` returns at once and NDI reads the
//!   buffer until the next send, so the previous frame is held until then.

use std::ffi::{c_int, CString};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::ffi::*;
use super::runtime::Runtime;
use super::NdiError;
use crate::core::frame_out::{Frame, Mailbox, PixelFormat, Samples};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendMode {
    Sync,
    #[default]
    Async,
}

/// What a sender reports.
#[derive(Default)]
pub struct SenderStats {
    pub sent: AtomicU64,
    /// Frames not sent because no receiver was connected.
    pub skipped: AtomicU64,
    /// Receivers connected (−1 until first asked).
    pub receivers: AtomicI32,
    /// Bit 0: on program in some receiver, bit 1: on preview.
    pub tally: AtomicU8,
    /// More than 1 % of the frames of the last second were dropped before sending.
    pub dropping: AtomicBool,
    /// Time spent in each send call (enable for measurements).
    pub send_time: Samples,
    /// Time from the tick a frame was made for to its send call returning (enable likewise).
    pub tick_to_sent: Samples,
}

impl SenderStats {
    pub fn tally(&self) -> Tally {
        let t = self.tally.load(Ordering::Relaxed);
        Tally {
            on_program: t & 1 != 0,
            on_preview: t & 2 != 0,
        }
    }
}

pub struct NdiSender {
    pub stats: Arc<SenderStats>,
    stop: Arc<AtomicBool>,
    mailbox: Arc<dyn FrameInbox>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The mailbox a sender takes frames from (the frame source's outlet, or a plain mailbox).
pub trait FrameInbox: Send + Sync {
    fn mailbox(&self) -> &Mailbox<Arc<Frame>>;
    /// Frames replaced before the sender took them (an outlet counts them).
    fn dropped(&self) -> u64 {
        0
    }
    /// Whether the sender wants frames now (an outlet passes it to its source).
    fn set_wanted(&self, _wanted: bool) {}
}

impl FrameInbox for Mailbox<Arc<Frame>> {
    fn mailbox(&self) -> &Mailbox<Arc<Frame>> {
        self
    }
}

#[cfg(windows)]
impl FrameInbox for crate::frames::Outlet {
    fn mailbox(&self) -> &Mailbox<Arc<Frame>> {
        &self.mailbox
    }
    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
    fn set_wanted(&self, wanted: bool) {
        self.wanted.store(wanted, Ordering::Relaxed);
    }
}

struct Instance(SendInstance);
// SAFETY: an NDI sender may be used from any thread; only the sender thread uses it after start.
unsafe impl Send for Instance {}

impl NdiSender {
    /// Creates the NDI source `name` (shown as `MACHINE (name)`) in `groups` (None = default)
    /// and starts sending what arrives in `inbox`.
    pub fn start(
        runtime: Arc<Runtime>,
        name: &str,
        groups: Option<&str>,
        inbox: Arc<dyn FrameInbox>,
        mode: SendMode,
        on_change: Option<OnChange>,
    ) -> Result<Self, NdiError> {
        let c_name = CString::new(name).map_err(|_| NdiError::Name)?;
        let c_groups = groups
            .map(CString::new)
            .transpose()
            .map_err(|_| NdiError::Name)?;
        let create = SendCreate {
            p_ndi_name: c_name.as_ptr(),
            p_groups: c_groups.as_ref().map_or(std::ptr::null(), |g| g.as_ptr()),
            // Kinetra's output clock paces; NDI must not sleep in the send call.
            clock_video: false,
            clock_audio: false,
        };
        let inst = unsafe { (runtime.api.send_create)(&create) };
        if inst.is_null() {
            return Err(NdiError::SendCreate);
        }
        let stats = Arc::new(SenderStats {
            receivers: AtomicI32::new(-1),
            ..Default::default()
        });
        let stop = Arc::new(AtomicBool::new(false));
        let job = Job {
            runtime,
            name: name.to_string(),
            inst: Instance(inst),
            inbox: inbox.clone(),
            stats: stats.clone(),
            stop: stop.clone(),
            mode,
            on_change,
        };
        let thread = std::thread::Builder::new()
            .name("ndi-send".into())
            .spawn(move || job.run())
            .map_err(|_| NdiError::SendCreate)?;
        Ok(Self {
            stats,
            stop,
            mailbox: inbox,
            thread: Some(thread),
        })
    }
}

impl Drop for NdiSender {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wakes the thread if it waits for a frame.
        self.mailbox.mailbox().close();
        if let Some(t) = self.thread.take() {
            t.join().ok();
        }
    }
}

struct Job {
    runtime: Arc<Runtime>,
    name: String,
    inst: Instance,
    inbox: Arc<dyn FrameInbox>,
    stats: Arc<SenderStats>,
    stop: Arc<AtomicBool>,
    mode: SendMode,
    on_change: Option<OnChange>,
}

/// Called (on the sender thread) when the receiver count, tally or dropping changes.
pub type OnChange = Arc<dyn Fn() + Send + Sync>;

impl Job {
    /// What the UI shows: (receivers, tally bits, dropping).
    fn snapshot(&self) -> (i32, u8, bool) {
        let s = &self.stats;
        (
            s.receivers.load(Ordering::Relaxed),
            s.tally.load(Ordering::Relaxed),
            s.dropping.load(Ordering::Relaxed),
        )
    }

    fn run(self) {
        let api = self.runtime.api;
        let inst = self.inst.0;
        // Async: the frame NDI may still read until the next send call.
        let mut in_flight: Option<Arc<Frame>> = None;
        let mut polled = Instant::now() - Duration::from_secs(1);
        let mut shown = self.snapshot();
        // (dropped, sent) at the last poll, for the dropping rate.
        let mut counts = (self.inbox.dropped(), 0u64);
        #[cfg(windows)]
        let _boost = crate::frames::clock::boost_thread();
        while !self.stop.load(Ordering::Relaxed) {
            // NDI encodes every frame it's given, receivers or not (measured: 2.2 ms a 1080p
            // frame either way). With nobody connected the outlet is told it isn't wanted (the
            // source then does no GPU work and hands out nothing) and the receiver count is asked
            // every 20 ms instead, so a new receiver gets a picture within a few ticks.
            let idle = self.stats.receivers.load(Ordering::Relaxed) <= 0;
            let wait = Duration::from_millis(if idle { 20 } else { 500 });
            let mut frame = self.inbox.mailbox().take(wait);
            if idle {
                let n: c_int = unsafe { (api.send_get_no_connections)(inst, 0) };
                self.stats.receivers.store(n, Ordering::Relaxed);
                if n <= 0 && frame.take().is_some() {
                    self.stats.skipped.fetch_add(1, Ordering::Relaxed);
                }
            }
            self.inbox
                .set_wanted(self.stats.receivers.load(Ordering::Relaxed) > 0);
            if let Some(f) = frame {
                let v = video_frame(&f);
                let t0 = Instant::now();
                match self.mode {
                    SendMode::Sync => unsafe { (api.send_video_v2)(inst, &v) },
                    SendMode::Async => unsafe { (api.send_video_async_v2)(inst, &v) },
                }
                let spent = t0.elapsed();
                if self.mode == SendMode::Async {
                    // The previous frame is released now that NDI moved on to this one.
                    in_flight = Some(f.clone());
                }
                self.stats.send_time.push(spent);
                #[cfg(windows)]
                {
                    let late = crate::frames::clock::now_us() - f.due_us;
                    self.stats
                        .tick_to_sent
                        .push(Duration::from_micros(late.max(0) as u64));
                }
                self.stats.sent.fetch_add(1, Ordering::Relaxed);
            }
            if polled.elapsed() >= Duration::from_secs(1) {
                polled = Instant::now();
                let n: c_int = unsafe { (api.send_get_no_connections)(inst, 0) };
                self.stats.receivers.store(n, Ordering::Relaxed);
                let mut t = Tally::default();
                unsafe { (api.send_get_tally)(inst, &mut t, 0) };
                self.stats.tally.store(
                    u8::from(t.on_program) | (u8::from(t.on_preview) << 1),
                    Ordering::Relaxed,
                );
                let (dropped, sent) = (
                    self.inbox.dropped(),
                    self.stats.sent.load(Ordering::Relaxed),
                );
                let (dd, ds) = (dropped - counts.0, sent - counts.1);
                counts = (dropped, sent);
                self.stats
                    .dropping
                    .store(dd * 100 > dd + ds, Ordering::Relaxed);
            }
            let now = self.snapshot();
            if now != shown {
                if now.0 != shown.0 && now.0 >= 0 {
                    // The count NDI reports (one receiver has read as 2): for checking it against
                    // vMix and Studio Monitor before the UI shows a number.
                    tracing::info!("NDI source {}: {} connection(s)", self.name, now.0);
                }
                shown = now;
                if let Some(f) = &self.on_change {
                    f();
                }
            }
            if self.inbox.mailbox().is_closed() {
                break;
            }
        }
        // Destroying the sender is a synchronizing event: NDI is done with `in_flight` after it.
        unsafe { (api.send_destroy)(inst) };
        drop(in_flight);
    }
}

fn video_frame(f: &Frame) -> VideoFrameV2 {
    VideoFrameV2 {
        xres: f.width as c_int,
        yres: f.height as c_int,
        fourcc: match f.format {
            PixelFormat::Uyva => FOURCC_UYVA,
            PixelFormat::Uyvy => FOURCC_UYVY,
            PixelFormat::Bgra => FOURCC_BGRA,
        },
        frame_rate_n: f.rate.num as c_int,
        frame_rate_d: f.rate.den as c_int,
        picture_aspect_ratio: 0.0,
        frame_format_type: FRAME_FORMAT_PROGRESSIVE,
        timecode: TIMECODE_SYNTHESIZE,
        // NDI only reads it (the header's pointer isn't const).
        p_data: f.data.as_ptr() as *mut u8,
        line_stride_in_bytes: f.stride() as c_int,
        p_metadata: std::ptr::null(),
        timestamp: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::frame_out::{AlphaMode, FrameRate, Pool};
    use crate::ndi::runtime::Api;
    use std::ffi::{c_char, c_void};
    use std::sync::Mutex;

    /// What the fake runtime saw.
    #[derive(Default, Debug, Clone)]
    struct Seen {
        initialized: u32,
        destroyed: u32,
        created: Vec<(String, bool)>,
        sends: Vec<(u32, i32, i32, i32, i32, i32, bool)>,
        sender_destroyed: u32,
        polls: u32,
    }

    static SEEN: Mutex<Option<Seen>> = Mutex::new(None);
    static SERIAL: Mutex<()> = Mutex::new(());
    static CONNECTIONS: AtomicI32 = AtomicI32::new(2);

    fn seen() -> Seen {
        SEEN.lock().unwrap().clone().unwrap_or_default()
    }
    fn with(f: impl FnOnce(&mut Seen)) {
        f(SEEN.lock().unwrap().get_or_insert_with(Seen::default))
    }

    unsafe extern "C" fn initialize() -> bool {
        with(|s| s.initialized += 1);
        true
    }
    unsafe extern "C" fn destroy() {
        with(|s| s.destroyed += 1);
    }
    unsafe extern "C" fn version() -> *const c_char {
        c"FAKE 6.3".as_ptr()
    }
    unsafe extern "C" fn cpu_ok() -> bool {
        true
    }
    unsafe extern "C" fn send_create(c: *const SendCreate) -> SendInstance {
        let c = unsafe { &*c };
        let name = unsafe { std::ffi::CStr::from_ptr(c.p_ndi_name) }
            .to_string_lossy()
            .into_owned();
        with(|s| s.created.push((name, c.clock_video)));
        0x1 as SendInstance
    }
    unsafe extern "C" fn send_destroy(_: SendInstance) {
        with(|s| s.sender_destroyed += 1);
    }
    unsafe extern "C" fn send_video(_: SendInstance, v: *const VideoFrameV2) {
        let v = unsafe { &*v };
        let first = unsafe { *v.p_data };
        with(|s| {
            s.sends.push((
                v.fourcc,
                v.xres,
                v.yres,
                v.frame_rate_n,
                v.frame_rate_d,
                v.line_stride_in_bytes,
                first == 7,
            ))
        });
    }
    unsafe extern "C" fn tally(_: SendInstance, t: *mut Tally, _: u32) -> bool {
        unsafe {
            *t = Tally {
                on_program: true,
                on_preview: false,
            }
        };
        true
    }
    unsafe extern "C" fn connections(_: SendInstance, _: u32) -> c_int {
        with(|s| s.polls += 1);
        CONNECTIONS.load(Ordering::Relaxed)
    }
    unsafe extern "C" fn find_create(_: *const FindCreate) -> FindInstance {
        std::ptr::null_mut()
    }
    unsafe extern "C" fn find_destroy(_: FindInstance) {}
    unsafe extern "C" fn find_wait(_: FindInstance, _: u32) -> bool {
        false
    }
    unsafe extern "C" fn find_sources(_: FindInstance, n: *mut u32) -> *const Source {
        unsafe { *n = 0 };
        std::ptr::null()
    }
    unsafe extern "C" fn recv_create(_: *const RecvCreateV3) -> RecvInstance {
        std::ptr::null_mut()
    }
    unsafe extern "C" fn recv_destroy(_: RecvInstance) {}
    unsafe extern "C" fn recv_capture(
        _: RecvInstance,
        _: *mut VideoFrameV2,
        _: *mut c_void,
        _: *mut c_void,
        _: u32,
    ) -> c_int {
        FRAME_TYPE_NONE
    }
    unsafe extern "C" fn recv_free(_: RecvInstance, _: *const VideoFrameV2) {}
    unsafe extern "C" fn recv_perf(
        _: RecvInstance,
        _: *mut RecvPerformance,
        _: *mut RecvPerformance,
    ) {
    }

    static FAKE: Api = Api {
        initialize,
        destroy,
        version,
        is_supported_cpu: cpu_ok,
        send_create,
        send_destroy,
        send_video_v2: send_video,
        send_video_async_v2: send_video,
        send_get_tally: tally,
        send_get_no_connections: connections,
        find_create_v2: find_create,
        find_destroy,
        find_wait_for_sources: find_wait,
        find_get_current_sources: find_sources,
        recv_create_v3: recv_create,
        recv_destroy,
        recv_capture_v2: recv_capture,
        recv_free_video_v2: recv_free,
        recv_get_performance: recv_perf,
    };

    fn frame(pool: &Arc<Pool>, tick: i64) -> Arc<Frame> {
        let mut data = pool.take();
        data[0] = 7;
        Arc::new(Frame {
            data,
            width: 4,
            height: 2,
            format: PixelFormat::Uyva,
            alpha: AlphaMode::Straight,
            rate: FrameRate::R59_94,
            tick,
            due_us: 0,
        })
    }

    fn wait_until(mut ok: impl FnMut() -> bool) {
        let t = Instant::now();
        while !ok() {
            assert!(
                t.elapsed() < Duration::from_secs(5),
                "timed out; saw {:?}",
                seen()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn sends_frames_as_uyva_and_reports_receivers() {
        let _g = SERIAL.lock().unwrap();
        *SEEN.lock().unwrap() = None;
        let rt = Arc::new(Runtime::start(&FAKE).unwrap());
        assert_eq!(rt.version, "FAKE 6.3");
        let inbox: Arc<Mailbox<Arc<Frame>>> = Arc::default();
        let pool = Pool::new(PixelFormat::Uyva.frame_bytes(4, 2), 4);
        let s = NdiSender::start(
            rt.clone(),
            "Kinetra Main",
            None,
            inbox.clone(),
            SendMode::Sync,
            None,
        )
        .unwrap();
        inbox.put(frame(&pool, 1));
        wait_until(|| s.stats.sent.load(Ordering::Relaxed) == 1);
        wait_until(|| s.stats.receivers.load(Ordering::Relaxed) == 2);
        assert!(s.stats.tally().on_program);
        drop(s);
        let seen = seen();
        assert_eq!(
            seen.created,
            [("Kinetra Main".to_string(), false)],
            "never clocked by NDI"
        );
        assert_eq!(seen.sends, [(FOURCC_UYVA, 4, 2, 60000, 1001, 8, true)]);
        assert_eq!(seen.sender_destroyed, 1);
        drop(rt);
        assert_eq!((seen.initialized, self::seen().destroyed), (1, 1));
    }

    #[test]
    fn async_holds_the_previous_frame_until_the_next_send() {
        let _g = SERIAL.lock().unwrap();
        *SEEN.lock().unwrap() = None;
        let rt = Arc::new(Runtime::start(&FAKE).unwrap());
        let inbox: Arc<Mailbox<Arc<Frame>>> = Arc::default();
        let pool = Pool::new(PixelFormat::Uyva.frame_bytes(4, 2), 4);
        let s = NdiSender::start(
            rt,
            "A",
            Some("Studio"),
            inbox.clone(),
            SendMode::Async,
            None,
        )
        .unwrap();
        inbox.put(frame(&pool, 1));
        wait_until(|| s.stats.sent.load(Ordering::Relaxed) == 1);
        assert_eq!(pool.idle(), 0, "NDI may still read the first frame");
        inbox.put(frame(&pool, 2));
        wait_until(|| s.stats.sent.load(Ordering::Relaxed) == 2);
        wait_until(|| pool.idle() == 1);
        drop(s);
        assert_eq!(
            pool.idle(),
            2,
            "everything back after the sender is destroyed"
        );
    }

    #[test]
    fn nothing_is_sent_while_no_receiver_is_connected() {
        let _g = SERIAL.lock().unwrap();
        *SEEN.lock().unwrap() = None;
        CONNECTIONS.store(0, Ordering::Relaxed);
        let rt = Arc::new(Runtime::start(&FAKE).unwrap());
        let inbox: Arc<Mailbox<Arc<Frame>>> = Arc::default();
        let pool = Pool::new(PixelFormat::Uyva.frame_bytes(4, 2), 4);
        let s = NdiSender::start(rt, "Idle", None, inbox.clone(), SendMode::Sync, None).unwrap();
        for t in 0..3 {
            inbox.put(frame(&pool, t));
            wait_until(|| s.stats.skipped.load(Ordering::Relaxed) > t as u64);
        }
        assert_eq!(s.stats.sent.load(Ordering::Relaxed), 0);
        assert!(seen().sends.is_empty(), "nothing encoded for nobody");
        // A receiver connects: the very next frame goes out.
        CONNECTIONS.store(1, Ordering::Relaxed);
        inbox.put(frame(&pool, 3));
        wait_until(|| s.stats.sent.load(Ordering::Relaxed) == 1);
        assert_eq!(s.stats.receivers.load(Ordering::Relaxed), 1);
        drop(s);
        CONNECTIONS.store(2, Ordering::Relaxed);
    }

    /// An inbox that records whether the sender wants frames (like the frame source's outlet).
    #[derive(Default)]
    struct WantInbox {
        mailbox: Mailbox<Arc<Frame>>,
        wanted: AtomicBool,
    }

    impl FrameInbox for WantInbox {
        fn mailbox(&self) -> &Mailbox<Arc<Frame>> {
            &self.mailbox
        }
        fn set_wanted(&self, wanted: bool) {
            self.wanted.store(wanted, Ordering::Relaxed);
        }
    }

    #[test]
    fn the_source_is_told_to_rest_while_nobody_receives() {
        let _g = SERIAL.lock().unwrap();
        *SEEN.lock().unwrap() = None;
        CONNECTIONS.store(0, Ordering::Relaxed);
        let rt = Arc::new(Runtime::start(&FAKE).unwrap());
        let inbox = Arc::new(WantInbox {
            wanted: AtomicBool::new(true),
            ..Default::default()
        });
        let s = NdiSender::start(rt, "Rest", None, inbox.clone(), SendMode::Sync, None).unwrap();
        // No frame arrives (the source rests), yet the count is asked every 20 ms.
        wait_until(|| !inbox.wanted.load(Ordering::Relaxed));
        let polls = seen().polls;
        wait_until(|| seen().polls >= polls + 3);
        // A receiver connects: wanted again, without any frame having been sent.
        CONNECTIONS.store(1, Ordering::Relaxed);
        wait_until(|| inbox.wanted.load(Ordering::Relaxed));
        assert_eq!(s.stats.sent.load(Ordering::Relaxed), 0);
        drop(s);
        CONNECTIONS.store(2, Ordering::Relaxed);
    }

    #[test]
    fn changes_are_reported_only_when_they_happen() {
        let _g = SERIAL.lock().unwrap();
        *SEEN.lock().unwrap() = None;
        let rt = Arc::new(Runtime::start(&FAKE).unwrap());
        let inbox: Arc<Mailbox<Arc<Frame>>> = Arc::default();
        let calls = Arc::new(AtomicU64::new(0));
        let c = calls.clone();
        let on_change: OnChange = Arc::new(move || {
            c.fetch_add(1, Ordering::Relaxed);
        });
        let s = NdiSender::start(
            rt,
            "Live",
            None,
            inbox.clone(),
            SendMode::Sync,
            Some(on_change),
        )
        .unwrap();
        // The first poll finds 2 receivers and program tally: one report.
        wait_until(|| calls.load(Ordering::Relaxed) == 1);
        let pool = Pool::new(PixelFormat::Uyva.frame_bytes(4, 2), 4);
        for t in 0..3 {
            inbox.put(frame(&pool, t));
            wait_until(|| s.stats.sent.load(Ordering::Relaxed) > t as u64);
        }
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "nothing changed while sending"
        );
        assert!(!s.stats.dropping.load(Ordering::Relaxed));
        drop(s);
    }

    #[test]
    fn invalid_names_are_refused() {
        let _g = SERIAL.lock().unwrap();
        let rt = Arc::new(Runtime::start(&FAKE).unwrap());
        let inbox: Arc<Mailbox<Arc<Frame>>> = Arc::default();
        assert!(matches!(
            NdiSender::start(rt, "a\0b", None, inbox, SendMode::Sync, None),
            Err(NdiError::Name)
        ));
    }
}
