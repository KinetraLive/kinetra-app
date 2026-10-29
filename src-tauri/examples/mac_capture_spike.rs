//! macOS capture spike (kinetra-private `docs/MACOS_PARITY_PLAN.md`, phase M1): can Kinetra get
//! frames with alpha from a WKWebView it doesn't show, fast enough for NDI at 1080p50?
//!
//! One test page (transparent background, a 50 % red box, an opaque green box, a box moved by a
//! CSS animation, a requestAnimationFrame counter) in a borderless window, placed five ways:
//! - `visible`: on the main display, in front;
//! - `offscreen`: far outside every display;
//! - `behindDesktop`: at the desktop picture's window level, ordered behind it;
//! - `transparent`: on the display with the window's alpha at 0;
//! - `orderedOut`: created but never shown.
//!
//! For each placement it measures how often the page paints (requestAnimationFrame), then the
//! routes of plan §1.2:
//! - A, ScreenCaptureKit on that one window: frames a second, intervals, alpha at known points,
//!   CPU and memory of this process and WebKit's processes;
//! - B, `CALayer renderInContext:` of the web view's layer (in process, no permission);
//! - C, `WKWebView takeSnapshot` (no permission; the remote preview fallback).
//!
//! Writes `report.json` and one PNG per capture into `--out` (default `mac-spike-out`). Run on a
//! GitHub macOS runner (`.github/workflows/macos-spike.yml`) or by a tester; it shows windows on
//! the main display for a few seconds (the `visible` and `transparent` placements).
//!
//!     cargo run --example mac_capture_spike -- --out spike-out

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("mac_capture_spike runs on macOS only");
}

#[cfg(target_os = "macos")]
fn main() {
    spike::run();
}

#[cfg(target_os = "macos")]
mod spike {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::{c_char, c_void, CStr};
    use std::path::{Path, PathBuf};
    use std::ptr::null_mut;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, Sel};
    use objc2::{define_class, msg_send, AllocAnyThread, ClassType, Encode, Encoding};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use serde_json::{json, Value};

    use kinetra_lib::frames::clock::now_us;

    const W: usize = 1920;
    const H: usize = 1080;

    const PAGE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><style>
html,body{margin:0;background:transparent;width:100%;height:100%;overflow:hidden}
#a{position:absolute;left:10%;top:10%;width:20%;height:20%;background:rgba(255,0,0,.5)}
#b{position:absolute;left:40%;top:10%;width:20%;height:20%;background:#00ff00}
#m{position:absolute;left:0;top:60%;width:10%;height:20%;background:#0000ff;animation:mv 2s linear infinite alternate}
@keyframes mv{to{transform:translateX(900%)}}
#t{position:absolute;left:70%;top:10%;font:64px sans-serif;color:#fff}
</style></head><body><div id="a"></div><div id="b"></div><div id="m"></div><div id="t">0</div>
<script>window.__f=0;function f(){window.__f++;if(window.__f%5==0)document.getElementById('t').textContent=window.__f;requestAnimationFrame(f)}requestAnimationFrame(f)</script>
</body></html>"#;

    // ---- C frameworks (declared by hand: a few functions each) ----

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGWindowLevelForKey(key: i32) -> i32;
        fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
        fn CGColorSpaceRelease(cs: *mut c_void);
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            space: *mut c_void,
            info: u32,
        ) -> *mut c_void;
        fn CGBitmapContextCreateImage(ctx: *mut c_void) -> *mut c_void;
        fn CGContextRelease(ctx: *mut c_void);
        fn CGContextDrawImage(ctx: *mut c_void, rect: NSRect, image: *mut c_void);
        fn CGImageGetWidth(image: *mut c_void) -> usize;
        fn CGImageGetHeight(image: *mut c_void) -> usize;
        fn CGImageRelease(image: *mut c_void);
    }

    #[link(name = "CoreMedia", kind = "framework")]
    extern "C" {
        fn CMSampleBufferGetImageBuffer(sample: *mut c_void) -> *mut c_void;
    }

    #[link(name = "CoreVideo", kind = "framework")]
    extern "C" {
        fn CVPixelBufferLockBaseAddress(pb: *mut c_void, flags: u64) -> i32;
        fn CVPixelBufferUnlockBaseAddress(pb: *mut c_void, flags: u64) -> i32;
        fn CVPixelBufferGetBaseAddress(pb: *mut c_void) -> *mut u8;
        fn CVPixelBufferGetBytesPerRow(pb: *mut c_void) -> usize;
        fn CVPixelBufferGetWidth(pb: *mut c_void) -> usize;
        fn CVPixelBufferGetHeight(pb: *mut c_void) -> usize;
    }

    #[link(name = "ScreenCaptureKit", kind = "framework")]
    extern "C" {}
    #[link(name = "WebKit", kind = "framework")]
    extern "C" {}
    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}
    #[link(name = "QuartzCore", kind = "framework")]
    extern "C" {}

    #[link(name = "objc")]
    extern "C" {
        fn objc_msgSend();
        fn objc_getProtocol(name: *const c_char) -> *mut c_void;
        fn class_addProtocol(cls: *mut c_void, protocol: *mut c_void) -> bool;
    }

    /// `kCGImageAlphaPremultipliedLast`: RGBA bytes, premultiplied.
    const RGBA_PREMULTIPLIED: u32 = 1;
    /// `kCGDesktopWindowLevelKey`.
    const DESKTOP_LEVEL_KEY: i32 = 2;
    /// `kCVPixelFormatType_32BGRA`.
    const BGRA: u32 = 0x4247_5241;
    /// `kCVPixelBufferLock_ReadOnly`.
    const LOCK_READ_ONLY: u64 = 1;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CMTime {
        value: i64,
        timescale: i32,
        flags: u32,
        epoch: i64,
    }

    // SAFETY: CMTime's C layout (CoreMedia/CMTime.h).
    unsafe impl Encode for CMTime {
        const ENCODING: Encoding = Encoding::Struct(
            "?",
            &[
                Encoding::LongLong,
                Encoding::Int,
                Encoding::UInt,
                Encoding::LongLong,
            ],
        );
    }

    // ---- helpers ----

    fn class(name: &CStr) -> &'static AnyClass {
        AnyClass::get(name).unwrap_or_else(|| panic!("class {name:?} missing"))
    }

    fn ns(s: &str) -> Retained<NSString> {
        NSString::from_str(s)
    }

    fn responds(obj: &AnyObject, sel: &CStr) -> bool {
        unsafe { msg_send![obj, respondsToSelector: Sel::register(sel)] }
    }

    fn error_text(err: *mut AnyObject) -> String {
        if err.is_null() {
            return String::new();
        }
        let s: Option<Retained<NSString>> = unsafe { msg_send![err, localizedDescription] };
        s.map(|s| s.to_string()).unwrap_or_default()
    }

    /// Runs the main run loop for `secs` (WebKit, AppKit and completion handlers need it).
    fn pump(secs: f64) {
        let end = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < end {
            autoreleasepool(|_| unsafe {
                let date: Retained<AnyObject> =
                    msg_send![class(c"NSDate"), dateWithTimeIntervalSinceNow: 0.01f64];
                let rl: Retained<AnyObject> = msg_send![class(c"NSRunLoop"), currentRunLoop];
                let _: () = msg_send![&*rl, runUntilDate: &*date];
            });
        }
    }

    /// Pumps until `done` or `timeout` seconds; true if done.
    fn pump_until(timeout: f64, done: impl Fn() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs_f64(timeout);
        while Instant::now() < end {
            if done() {
                return true;
            }
            pump(0.005);
        }
        done()
    }

    /// An RGBA (premultiplied) image.
    struct Img {
        w: usize,
        h: usize,
        rgba: Vec<u8>,
    }

    impl Img {
        fn from_cgimage(cg: *mut c_void) -> Option<Img> {
            if cg.is_null() {
                return None;
            }
            let (w, h) = unsafe { (CGImageGetWidth(cg), CGImageGetHeight(cg)) };
            let mut img = Img {
                w,
                h,
                rgba: vec![0; w * h * 4],
            };
            img.with_context(|ctx| unsafe {
                CGContextDrawImage(ctx, rect(0.0, 0.0, w as f64, h as f64), cg)
            });
            Some(img)
        }

        /// A CGBitmapContext drawing into this image's bytes.
        fn with_context(&mut self, f: impl FnOnce(*mut c_void)) {
            unsafe {
                let cs = CGColorSpaceCreateDeviceRGB();
                let ctx = CGBitmapContextCreate(
                    self.rgba.as_mut_ptr().cast(),
                    self.w,
                    self.h,
                    8,
                    self.w * 4,
                    cs,
                    RGBA_PREMULTIPLIED,
                );
                CGColorSpaceRelease(cs);
                if !ctx.is_null() {
                    f(ctx);
                    CGContextRelease(ctx);
                }
            }
        }

        fn px(&self, fx: f64, fy: f64) -> [u8; 4] {
            let x = ((fx * self.w as f64) as usize).min(self.w.saturating_sub(1));
            let y = ((fy * self.h as f64) as usize).min(self.h.saturating_sub(1));
            let i = (y * self.w + x) * 4;
            [
                self.rgba[i],
                self.rgba[i + 1],
                self.rgba[i + 2],
                self.rgba[i + 3],
            ]
        }

        /// Alpha at the known points (both orientations: layer rendering may be flipped) and
        /// overall: what proves the capture keeps transparency.
        fn analyze(&self) -> Value {
            let (mut transparent, mut opaque, mut partial) = (0usize, 0usize, 0usize);
            for &a in self.rgba.iter().skip(3).step_by(4) {
                match a {
                    0 => transparent += 1,
                    255 => opaque += 1,
                    _ => partial += 1,
                }
            }
            let n = (self.w * self.h).max(1) as f64;
            json!({
                "size": [self.w, self.h],
                "background": self.px(0.05, 0.05),
                "red50": self.px(0.2, 0.2),
                "green": self.px(0.5, 0.2),
                "red50Flipped": self.px(0.2, 0.8),
                "greenFlipped": self.px(0.5, 0.8),
                "alphaShare": {
                    "transparent": transparent as f64 / n,
                    "opaque": opaque as f64 / n,
                    "partial": partial as f64 / n,
                },
                "expected": "background alpha 0, red50 ≈ [128,0,0,128] (premultiplied), green [0,255,0,255]",
            })
        }

        fn hash(&self) -> u64 {
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            for b in self.rgba.iter().step_by(97) {
                h = (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
            }
            h
        }

        fn save_png(&mut self, path: &Path) -> Result<(), String> {
            let mut cg = null_mut();
            self.with_context(|ctx| cg = unsafe { CGBitmapContextCreateImage(ctx) });
            if cg.is_null() {
                return Err("no image".into());
            }
            let ok = autoreleasepool(|_| unsafe {
                let rep: objc2::rc::Allocated<AnyObject> =
                    msg_send![class(c"NSBitmapImageRep"), alloc];
                let rep: Option<Retained<AnyObject>> = msg_send![rep, initWithCGImage: cg];
                let Some(rep) = rep else { return false };
                let props: Retained<AnyObject> = msg_send![class(c"NSDictionary"), dictionary];
                // NSBitmapImageFileTypePNG
                let data: Option<Retained<AnyObject>> =
                    msg_send![&*rep, representationUsingType: 4usize, properties: &*props];
                let Some(data) = data else { return false };
                let p = ns(&path.to_string_lossy());
                msg_send![&*data, writeToFile: &*p, atomically: true]
            });
            unsafe { CGImageRelease(cg) };
            if ok {
                Ok(())
            } else {
                Err("PNG write failed".into())
            }
        }
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    /// Cumulative CPU seconds and resident memory of this process and WebKit's processes, from
    /// `ps` (pid → (cpu s, rss KB, name)).
    fn processes() -> HashMap<u32, (f64, u64, String)> {
        let me = std::process::id();
        let out = std::process::Command::new("ps")
            .args(["-axo", "pid=,time=,rss=,comm="])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let mut map = HashMap::new();
        for line in out.lines() {
            let mut it = line.split_whitespace();
            let (Some(pid), Some(time), Some(rss)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            let comm: String = it.collect::<Vec<_>>().join(" ");
            let Ok(pid) = pid.parse::<u32>() else {
                continue;
            };
            if pid != me && !comm.contains("WebKit") {
                continue;
            }
            let secs = time.split(':').fold(0.0, |acc, part| {
                acc * 60.0 + part.parse::<f64>().unwrap_or(0.0)
            });
            let name = if pid == me {
                "spike".to_string()
            } else {
                comm.rsplit('/').next().unwrap_or(&comm).to_string()
            };
            map.insert(pid, (secs, rss.parse().unwrap_or(0), name));
        }
        map
    }

    /// CPU cores used between two `processes()` samples, and the memory at the end, per process.
    fn usage(
        a: &HashMap<u32, (f64, u64, String)>,
        b: &HashMap<u32, (f64, u64, String)>,
        secs: f64,
    ) -> Value {
        let mut rows = Vec::new();
        let (mut cores, mut mb) = (0.0, 0.0);
        for (pid, (cpu, rss, name)) in b {
            let before = a.get(pid).map_or(0.0, |x| x.0);
            let c = (cpu - before).max(0.0) / secs;
            let m = *rss as f64 / 1024.0;
            cores += c;
            mb += m;
            rows.push(json!({"pid": pid, "name": name, "cores": c, "rssMB": m}));
        }
        json!({"totalCores": cores, "totalRssMB": mb, "processes": rows})
    }

    // ---- ScreenCaptureKit stream output ----

    struct SckState {
        frames: u64,
        empty: u64,
        stamps: Vec<i64>,
        grab: bool,
        grabbed: Option<Img>,
        size: (usize, usize),
    }

    static SCK: Mutex<SckState> = Mutex::new(SckState {
        frames: 0,
        empty: 0,
        stamps: Vec::new(),
        grab: false,
        grabbed: None,
        size: (0, 0),
    });

    fn sck() -> std::sync::MutexGuard<'static, SckState> {
        SCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A captured BGRA pixel buffer as RGBA.
    unsafe fn copy_bgra(pb: *mut c_void) -> Img {
        CVPixelBufferLockBaseAddress(pb, LOCK_READ_ONLY);
        let (w, h) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb));
        let bpr = CVPixelBufferGetBytesPerRow(pb);
        let base = CVPixelBufferGetBaseAddress(pb);
        let mut rgba = vec![0u8; w * h * 4];
        if !base.is_null() {
            for y in 0..h {
                let row = std::slice::from_raw_parts(base.add(y * bpr), w * 4);
                for x in 0..w {
                    let s = &row[x * 4..x * 4 + 4];
                    let d = &mut rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
                    d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
                }
            }
        }
        CVPixelBufferUnlockBaseAddress(pb, LOCK_READ_ONLY);
        Img { w, h, rgba }
    }

    fn on_sample(sample: *mut c_void, kind: isize) {
        if kind != 0 || sample.is_null() {
            return;
        }
        let pb = unsafe { CMSampleBufferGetImageBuffer(sample) };
        let mut s = sck();
        if pb.is_null() {
            // An idle / blank frame: nothing changed.
            s.empty += 1;
            return;
        }
        s.frames += 1;
        s.stamps.push(now_us());
        s.size = unsafe { (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb)) };
        // The first complete frame is kept (a still page sends no more); a later one replaces it
        // when asked.
        if s.grab || s.grabbed.is_none() {
            s.grab = false;
            s.grabbed = Some(unsafe { copy_bgra(pb) });
        }
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; no Drop.
        #[unsafe(super(NSObject))]
        #[name = "KinetraSpikeStreamOutput"]
        struct StreamOutput;

        impl StreamOutput {
            #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
            fn did_output(&self, _stream: *mut AnyObject, sample: *mut c_void, kind: isize) {
                on_sample(sample, kind);
            }
        }
    );

    /// A Send wrapper for objects handed over from completion handlers on other queues.
    struct Sendable(Retained<AnyObject>);
    // SAFETY: only moved to the main thread, which then owns it.
    unsafe impl Send for Sendable {}

    /// The SCWindow for `number`, from all windows (on screen or not).
    fn find_sc_window(number: isize) -> Result<Retained<AnyObject>, String> {
        let slot: Arc<Mutex<Option<Result<Sendable, String>>>> = Arc::default();
        let s2 = slot.clone();
        let block = RcBlock::new(move |content: *mut AnyObject, err: *mut AnyObject| {
            let r = match unsafe { Retained::retain(content) } {
                Some(c) => Ok(Sendable(c)),
                None => Err(format!("no shareable content: {}", error_text(err))),
            };
            *s2.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        });
        unsafe {
            let _: () = msg_send![
                class(c"SCShareableContent"),
                getShareableContentExcludingDesktopWindows: false,
                onScreenWindowsOnly: false,
                completionHandler: &*block
            ];
        }
        let done = || slot.lock().map(|s| s.is_some()).unwrap_or(true);
        if !pump_until(10.0, done) {
            return Err("shareable content: no answer in 10 s".into());
        }
        let content = slot
            .lock()
            .map_err(|_| "poisoned")?
            .take()
            .ok_or("no answer")??
            .0;
        unsafe {
            let windows: Retained<AnyObject> = msg_send![&*content, windows];
            let n: usize = msg_send![&*windows, count];
            for i in 0..n {
                let w: Retained<AnyObject> = msg_send![&*windows, objectAtIndex: i];
                let id: u32 = msg_send![&*w, windowID];
                if i64::from(id) == number as i64 {
                    return Ok(w);
                }
            }
            Err(format!("our window isn't among the {n} shareable windows"))
        }
    }

    /// Route A: a ScreenCaptureKit stream of `window` for `secs`.
    fn route_sck(window: &AnyObject, out: &Path, tag: &str, secs: f64) -> Value {
        if AnyClass::get(c"SCStream").is_none() {
            return json!({"error": "ScreenCaptureKit unavailable (macOS < 12.3)"});
        }
        let number: isize = unsafe { msg_send![window, windowNumber] };
        let scw = match find_sc_window(number) {
            Ok(w) => w,
            Err(e) => return json!({"error": e}),
        };
        *sck() = SckState {
            frames: 0,
            empty: 0,
            stamps: Vec::new(),
            grab: false,
            grabbed: None,
            size: (0, 0),
        };
        unsafe {
            let filter: objc2::rc::Allocated<AnyObject> =
                msg_send![class(c"SCContentFilter"), alloc];
            let filter: Retained<AnyObject> =
                msg_send![filter, initWithDesktopIndependentWindow: &*scw];
            let config: Retained<AnyObject> = msg_send![class(c"SCStreamConfiguration"), new];
            let _: () = msg_send![&*config, setWidth: W];
            let _: () = msg_send![&*config, setHeight: H];
            let _: () = msg_send![&*config, setPixelFormat: BGRA];
            let interval = CMTime {
                value: 1,
                timescale: 60,
                flags: 1,
                epoch: 0,
            };
            let _: () = msg_send![&*config, setMinimumFrameInterval: interval];
            let _: () = msg_send![&*config, setShowsCursor: false];
            let _: () = msg_send![&*config, setQueueDepth: 5isize];
            let mut options = Vec::new();
            if responds(&config, c"setShouldBeOpaque:") {
                let _: () = msg_send![&*config, setShouldBeOpaque: false];
                options.push("shouldBeOpaque=false");
            }
            if responds(&config, c"setIgnoreShadowsSingleWindow:") {
                let _: () = msg_send![&*config, setIgnoreShadowsSingleWindow: true];
                options.push("ignoreShadowsSingleWindow=true");
            }
            let stream: objc2::rc::Allocated<AnyObject> = msg_send![class(c"SCStream"), alloc];
            let stream: Retained<AnyObject> = msg_send![
                stream,
                initWithFilter: &*filter,
                configuration: &*config,
                delegate: null_mut::<AnyObject>()
            ];
            let output: Retained<StreamOutput> = msg_send![StreamOutput::alloc(), init];
            // addStreamOutput:type:sampleHandlerQueue:error: (`type` is a Rust keyword, so the
            // message is sent directly with its C signature).
            type AddOutput = unsafe extern "C" fn(
                *mut AnyObject,
                Sel,
                *mut AnyObject,
                isize,
                *mut c_void,
                *mut *mut AnyObject,
            ) -> Bool;
            let add: AddOutput = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let mut err: *mut AnyObject = null_mut();
            let ok = add(
                Retained::as_ptr(&stream) as *mut AnyObject,
                Sel::register(c"addStreamOutput:type:sampleHandlerQueue:error:"),
                Retained::as_ptr(&output) as *mut StreamOutput as *mut AnyObject,
                0,
                null_mut(),
                &mut err,
            );
            if !ok.as_bool() {
                return json!({"error": format!("addStreamOutput: {}", error_text(err))});
            }
            let started: Arc<Mutex<Option<String>>> = Arc::default();
            let s2 = started.clone();
            let block = RcBlock::new(move |err: *mut AnyObject| {
                *s2.lock().unwrap_or_else(|e| e.into_inner()) = Some(error_text(err));
            });
            let _: () = msg_send![&*stream, startCaptureWithCompletionHandler: &*block];
            pump_until(10.0, || started.lock().map(|s| s.is_some()).unwrap_or(true));
            let start_err = started.lock().ok().and_then(|s| s.clone());
            match &start_err {
                None => {
                    return json!({"error": "startCapture: no answer in 10 s", "options": options})
                }
                Some(e) if !e.is_empty() => {
                    return json!({"error": format!("startCapture: {e}"), "options": options})
                }
                _ => {}
            }
            // Warm up, then measure.
            pump(1.0);
            {
                let mut s = sck();
                s.frames = 0;
                s.empty = 0;
                s.stamps.clear();
            }
            let before = processes();
            let t0 = Instant::now();
            pump(secs);
            let elapsed = t0.elapsed().as_secs_f64();
            let after = processes();
            let first = {
                let s = sck();
                (s.frames, s.empty, s.stamps.clone(), s.size)
            };
            sck().grab = true;
            let fresh = pump_until(2.0, || !sck().grab);
            let stopped: Arc<Mutex<bool>> = Arc::default();
            let s3 = stopped.clone();
            let stop = RcBlock::new(move |_err: *mut AnyObject| {
                *s3.lock().unwrap_or_else(|e| e.into_inner()) = true;
            });
            let _: () = msg_send![&*stream, stopCaptureWithCompletionHandler: &*stop];
            pump_until(5.0, || stopped.lock().map(|s| *s).unwrap_or(true));

            let (frames, empty, stamps, size) = first;
            let mut gaps: Vec<i64> = stamps.windows(2).map(|w| w[1] - w[0]).collect();
            gaps.sort_unstable();
            let pct = |p: f64| -> i64 {
                if gaps.is_empty() {
                    0
                } else {
                    gaps[((gaps.len() - 1) as f64 * p) as usize]
                }
            };
            let mut grabbed = sck().grabbed.take();
            let image = match grabbed.as_mut() {
                Some(img) => {
                    let file = out.join(format!("{tag}-sck.png"));
                    let saved = img.save_png(&file).err();
                    json!({"analysis": img.analyze(), "png": file.file_name().map(|f| f.to_string_lossy().into_owned()), "saveError": saved})
                }
                None => json!(null),
            };
            json!({
                "options": options,
                "framesPerSecond": frames as f64 / elapsed,
                "emptyPerSecond": empty as f64 / elapsed,
                "bufferSize": [size.0, size.1],
                "intervalUs": {"p50": pct(0.5), "p95": pct(0.95), "max": gaps.last().copied().unwrap_or(0)},
                "usage": usage(&before, &after, elapsed),
                "image": image,
                "imageAfterMeasuring": fresh,
            })
        }
    }

    /// Route C: `takeSnapshotWithConfiguration:` five times.
    fn route_snapshot(web: &AnyObject, out: &Path, tag: &str) -> Value {
        let mut times = Vec::new();
        let mut first: Option<Img> = None;
        let mut hashes = Vec::new();
        for _ in 0..5 {
            let slot: Rc<RefCell<Option<Result<Img, String>>>> = Rc::default();
            let s2 = slot.clone();
            let block = RcBlock::new(move |image: *mut AnyObject, err: *mut AnyObject| {
                let r = if image.is_null() {
                    Err(error_text(err))
                } else {
                    let cg: *mut c_void = unsafe {
                        msg_send![
                            image,
                            CGImageForProposedRect: null_mut::<NSRect>(),
                            context: null_mut::<AnyObject>(),
                            hints: null_mut::<AnyObject>()
                        ]
                    };
                    Img::from_cgimage(cg).ok_or_else(|| "no CGImage".to_string())
                };
                *s2.borrow_mut() = Some(r);
            });
            let t0 = Instant::now();
            unsafe {
                let _: () = msg_send![
                    web,
                    takeSnapshotWithConfiguration: null_mut::<AnyObject>(),
                    completionHandler: &*block
                ];
            }
            if !pump_until(5.0, || slot.borrow().is_some()) {
                return json!({"error": "no snapshot in 5 s"});
            }
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
            let result = slot.borrow_mut().take();
            match result {
                Some(Ok(img)) => {
                    hashes.push(img.hash());
                    if first.is_none() {
                        first = Some(img);
                    }
                }
                Some(Err(e)) => return json!({"error": e}),
                None => {}
            }
        }
        let image = first.as_mut().map(|img| {
            let file = out.join(format!("{tag}-snapshot.png"));
            let saved = img.save_png(&file).err();
            json!({"analysis": img.analyze(), "png": file.file_name().map(|f| f.to_string_lossy().into_owned()), "saveError": saved})
        });
        hashes.dedup();
        json!({"ms": times, "distinctPictures": hashes.len(), "image": image})
    }

    /// Route B: the web view's layer rendered in process.
    fn route_layer(web: &AnyObject, out: &Path, tag: &str) -> Value {
        let layer: Option<Retained<AnyObject>> = unsafe { msg_send![web, layer] };
        let Some(layer) = layer else {
            return json!({"error": "the web view has no layer"});
        };
        let sublayers: Option<Retained<AnyObject>> = unsafe { msg_send![&*layer, sublayers] };
        let sub_count: usize = sublayers
            .map(|s| unsafe { msg_send![&*s, count] })
            .unwrap_or(0);
        let mut img = Img {
            w: W,
            h: H,
            rgba: vec![0; W * H * 4],
        };
        let mut times = Vec::new();
        for _ in 0..3 {
            img.rgba.fill(0);
            let t0 = Instant::now();
            img.with_context(|ctx| unsafe {
                let _: () = msg_send![&*layer, renderInContext: ctx];
            });
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        let file = out.join(format!("{tag}-layer.png"));
        let saved = img.save_png(&file).err();
        let class_name: Retained<NSString> = unsafe { msg_send![&*layer, className] };
        json!({
            "ms": times,
            "layerClass": class_name.to_string(),
            "sublayers": sub_count,
            "analysis": img.analyze(),
            "png": file.file_name().map(|f| f.to_string_lossy().into_owned()),
            "saveError": saved,
        })
    }

    /// `window.__f` (requestAnimationFrame callbacks so far).
    fn raf_count(web: &AnyObject) -> Option<f64> {
        let slot: Rc<RefCell<Option<f64>>> = Rc::default();
        let s2 = slot.clone();
        let block = RcBlock::new(move |res: *mut AnyObject, _err: *mut AnyObject| {
            let v = if res.is_null() {
                f64::NAN
            } else {
                unsafe { msg_send![res, doubleValue] }
            };
            *s2.borrow_mut() = Some(v);
        });
        unsafe {
            let _: () = msg_send![
                web,
                evaluateJavaScript: &*ns("window.__f"),
                completionHandler: &*block
            ];
        }
        pump_until(5.0, || slot.borrow().is_some());
        let v = *slot.borrow();
        v.filter(|v| v.is_finite())
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Placement {
        Visible,
        Offscreen,
        BehindDesktop,
        Transparent,
        OrderedOut,
    }

    impl Placement {
        fn tag(self) -> &'static str {
            match self {
                Self::Visible => "visible",
                Self::Offscreen => "offscreen",
                Self::BehindDesktop => "behindDesktop",
                Self::Transparent => "transparent",
                Self::OrderedOut => "orderedOut",
            }
        }
    }

    /// The window and web view for one placement.
    fn make_page(
        p: Placement,
        scale: f64,
    ) -> (Retained<AnyObject>, Retained<AnyObject>, Vec<String>) {
        let mut notes = Vec::new();
        let (pw, ph) = (W as f64 / scale, H as f64 / scale);
        unsafe {
            let window: objc2::rc::Allocated<AnyObject> = msg_send![class(c"NSWindow"), alloc];
            // NSWindowStyleMaskBorderless (0), NSBackingStoreBuffered (2).
            let window: Retained<AnyObject> = msg_send![
                window,
                initWithContentRect: rect(0.0, 0.0, pw, ph),
                styleMask: 0usize,
                backing: 2usize,
                defer: false
            ];
            let _: () = msg_send![&*window, setReleasedWhenClosed: false];
            let _: () = msg_send![&*window, setOpaque: false];
            let clear: Retained<AnyObject> = msg_send![class(c"NSColor"), clearColor];
            let _: () = msg_send![&*window, setBackgroundColor: &*clear];
            let _: () = msg_send![&*window, setHasShadow: false];
            let _: () = msg_send![&*window, setIgnoresMouseEvents: true];

            let config: Retained<AnyObject> = msg_send![class(c"WKWebViewConfiguration"), new];
            let store: Retained<AnyObject> =
                msg_send![class(c"WKWebsiteDataStore"), nonPersistentDataStore];
            let _: () = msg_send![&*config, setWebsiteDataStore: &*store];
            let prefs: Retained<AnyObject> = msg_send![&*config, preferences];
            if responds(&prefs, c"setInactiveSchedulingPolicy:") {
                // WKInactiveSchedulingPolicyNone
                let _: () = msg_send![&*prefs, setInactiveSchedulingPolicy: 2isize];
                notes.push("inactiveSchedulingPolicy=none".into());
            } else {
                notes.push("inactiveSchedulingPolicy unavailable (macOS < 14)".into());
            }
            let web: objc2::rc::Allocated<AnyObject> = msg_send![class(c"WKWebView"), alloc];
            let web: Retained<AnyObject> = msg_send![
                web,
                initWithFrame: rect(0.0, 0.0, pw, ph),
                configuration: &*config
            ];
            let no: Retained<AnyObject> = msg_send![class(c"NSNumber"), numberWithBool: false];
            let _: () = msg_send![&*web, setValue: &*no, forKey: &*ns("drawsBackground")];
            let _: () = msg_send![&*web, setWantsLayer: true];
            // WebKit stops painting (requestAnimationFrame, CSS animations) in a window macOS
            // reports as occluded; this SPI (private, macOS 10.13+) keeps it painting.
            let keep = std::env::args().any(|a| a == "--keep-occlusion");
            if !keep && responds(&web, c"_setWindowOcclusionDetectionEnabled:") {
                let _: () = msg_send![&*web, _setWindowOcclusionDetectionEnabled: false];
                notes.push("windowOcclusionDetection=off".into());
            } else {
                notes.push("windowOcclusionDetection=on".into());
            }
            let _: () = msg_send![&*window, setContentView: &*web];

            match p {
                Placement::Visible => {
                    let _: () = msg_send![&*window, orderFrontRegardless];
                }
                Placement::Offscreen => {
                    let _: () = msg_send![&*window, setFrame: rect(-30000.0, -30000.0, pw, ph), display: true];
                    let _: () = msg_send![&*window, orderFrontRegardless];
                }
                Placement::BehindDesktop => {
                    let level = CGWindowLevelForKey(DESKTOP_LEVEL_KEY) as isize;
                    let _: () = msg_send![&*window, setLevel: level];
                    let _: () = msg_send![&*window, orderBack: null_mut::<AnyObject>()];
                    notes.push(format!("level={level}"));
                }
                Placement::Transparent => {
                    let _: () = msg_send![&*window, setAlphaValue: 0.0f64];
                    let _: () = msg_send![&*window, orderFrontRegardless];
                }
                Placement::OrderedOut => {}
            }
            let visible: bool = msg_send![&*window, isVisible];
            let occlusion: usize = msg_send![&*window, occlusionState];
            // NSWindowOcclusionStateVisible = 1 << 1
            notes.push(format!(
                "isVisible={visible} occlusionVisible={}",
                occlusion & 2 != 0
            ));
            let backing: f64 = msg_send![&*window, backingScaleFactor];
            notes.push(format!("backingScaleFactor={backing}"));

            let _: Option<Retained<AnyObject>> = msg_send![
                &*web,
                loadHTMLString: &*ns(PAGE),
                baseURL: null_mut::<AnyObject>()
            ];
            (window, web, notes)
        }
    }

    pub fn run() {
        let out = std::env::args()
            .skip_while(|a| a != "--out")
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("mac-spike-out"));
        let secs: f64 = std::env::args()
            .skip_while(|a| a != "--seconds")
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(4.0);
        if let Err(e) = std::fs::create_dir_all(&out) {
            eprintln!("can't create {}: {e}", out.display());
            std::process::exit(1);
        }
        // Register the output class and declare its protocol (ScreenCaptureKit may check it).
        unsafe {
            let cls = StreamOutput::class();
            let proto = objc_getProtocol(c"SCStreamOutput".as_ptr());
            if !proto.is_null() {
                class_addProtocol(cls as *const AnyClass as *mut c_void, proto);
            }
        }
        let mut report = serde_json::Map::new();
        let scale = unsafe {
            let app: Retained<AnyObject> = msg_send![class(c"NSApplication"), sharedApplication];
            // NSApplicationActivationPolicyAccessory: no Dock icon, no menu bar.
            let _: bool = msg_send![&*app, setActivationPolicy: 1isize];
            let _: () = msg_send![&*app, finishLaunching];
            let info: Retained<AnyObject> = msg_send![class(c"NSProcessInfo"), processInfo];
            let os: Retained<NSString> = msg_send![&*info, operatingSystemVersionString];
            report.insert("macos".into(), json!(os.to_string()));
            report.insert("arch".into(), json!(std::env::consts::ARCH));
            // App Nap off while measuring (NSActivityUserInitiated | NSActivityLatencyCritical).
            let activity: Retained<AnyObject> = msg_send![
                &*info,
                beginActivityWithOptions: 0x00FF_FFFFu64 | 0xFF_0000_0000u64,
                reason: &*ns("capture spike")
            ];
            std::mem::forget(activity);
            let screen: Option<Retained<AnyObject>> = msg_send![class(c"NSScreen"), mainScreen];
            match screen {
                Some(s) => {
                    let f: NSRect = msg_send![&*s, frame];
                    let scale: f64 = msg_send![&*s, backingScaleFactor];
                    report.insert(
                        "mainScreen".into(),
                        json!({"points": [f.size.width, f.size.height], "scale": scale}),
                    );
                    scale.max(1.0)
                }
                None => {
                    report.insert("mainScreen".into(), json!(null));
                    1.0
                }
            }
        };
        let permitted = unsafe { CGPreflightScreenCaptureAccess() };
        report.insert("screenRecordingPermitted".into(), json!(permitted));
        eprintln!("screen recording permitted: {permitted}");

        let mut placements = serde_json::Map::new();
        for p in [
            Placement::Visible,
            Placement::Offscreen,
            Placement::BehindDesktop,
            Placement::Transparent,
            Placement::OrderedOut,
        ] {
            eprintln!("placement {}", p.tag());
            let (window, web, notes) = make_page(p, scale);
            pump(2.5);
            let raf = {
                let a = raf_count(&web);
                let t0 = Instant::now();
                pump(3.0);
                let b = raf_count(&web);
                match (a, b) {
                    (Some(a), Some(b)) => json!((b - a) / t0.elapsed().as_secs_f64()),
                    _ => json!(null),
                }
            };
            let mut r = serde_json::Map::new();
            r.insert("notes".into(), json!(notes));
            let occlusion: usize = unsafe { msg_send![&*window, occlusionState] };
            r.insert(
                "occlusionVisibleAfterLoad".into(),
                json!(occlusion & 2 != 0),
            );
            r.insert("rafPerSecond".into(), raf);
            r.insert("sck".into(), route_sck(&window, &out, p.tag(), secs));
            if matches!(p, Placement::Visible | Placement::Offscreen) {
                r.insert("snapshot".into(), route_snapshot(&web, &out, p.tag()));
                r.insert("layer".into(), route_layer(&web, &out, p.tag()));
            }
            placements.insert(p.tag().into(), Value::Object(r));
            unsafe {
                let _: () = msg_send![&*window, orderOut: null_mut::<AnyObject>()];
                let _: () = msg_send![&*window, close];
            }
            drop(web);
            drop(window);
            pump(0.5);
        }
        report.insert("placements".into(), Value::Object(placements));
        let text = serde_json::to_string_pretty(&Value::Object(report)).unwrap_or_default();
        let file = out.join("report.json");
        if let Err(e) = std::fs::write(&file, &text) {
            eprintln!("can't write {}: {e}", file.display());
        }
        println!("{text}");
    }
}
