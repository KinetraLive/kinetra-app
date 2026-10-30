//! The macOS display backend: enumeration with CoreGraphics (active displays, mirrors left out),
//! names from NSScreen, and the `displays` watcher thread (a
//! [`LoopThread`](crate::core::runloop::LoopThread) with a CoreGraphics reconfiguration callback,
//! debounced like Windows').
//!
//! Positions and sizes are points in CoreGraphics' global display space: origin at the top-left
//! of the main display, y down. It's the one space that is consistent across displays with
//! different scale factors, and the one window positions (Tauri's logical positions) use.
//! - id: the display's UUID (`CGDisplayCreateUUIDFromDisplayID`): stable across reboots and
//!   ports for the same panel, like the Windows interface path;
//! - `edid_key`: vendor + model + serial from CoreGraphics (the EDID's values), in the Windows
//!   form (`DEL-A0B1-12345678`), so "moved to another port" works the same way;
//! - `refresh_hz`: the mode's rate (built-in panels report 0: 60);
//! - `scaled`: the mode's backing pixels differ from the panel's native mode (a "looks like"
//!   size the panel doesn't have; the system resamples).

use std::any::Any;
use std::ffi::{c_char, c_void, CStr};
use std::sync::Arc;

use super::{DisplayBackend, OnChange, Watch, DEBOUNCE};
use crate::core::model::DisplayInfo;
use crate::core::runloop::{Event, LoopThread, Waker};

type DisplayId = u32;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

type ReconfigurationCallback = extern "C" fn(DisplayId, u32, *mut c_void);

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(max: u32, displays: *mut DisplayId, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: DisplayId) -> CGRect;
    fn CGDisplayIsMain(display: DisplayId) -> u32;
    fn CGDisplayIsBuiltin(display: DisplayId) -> u32;
    fn CGDisplayMirrorsDisplay(display: DisplayId) -> DisplayId;
    fn CGDisplayVendorNumber(display: DisplayId) -> u32;
    fn CGDisplayModelNumber(display: DisplayId) -> u32;
    fn CGDisplaySerialNumber(display: DisplayId) -> u32;
    fn CGDisplayCopyDisplayMode(display: DisplayId) -> *mut c_void;
    fn CGDisplayCopyAllDisplayModes(display: DisplayId, options: *const c_void) -> *const c_void;
    fn CGDisplayModeGetPixelWidth(mode: *const c_void) -> usize;
    fn CGDisplayModeGetPixelHeight(mode: *const c_void) -> usize;
    fn CGDisplayModeGetRefreshRate(mode: *const c_void) -> f64;
    fn CGDisplayModeGetIOFlags(mode: *const c_void) -> u32;
    fn CGDisplayModeRelease(mode: *mut c_void);
    fn CGDisplayRegisterReconfigurationCallback(
        callback: ReconfigurationCallback,
        user_info: *mut c_void,
    ) -> i32;
    fn CGDisplayRemoveReconfigurationCallback(
        callback: ReconfigurationCallback,
        user_info: *mut c_void,
    ) -> i32;
}

#[link(name = "ColorSync", kind = "framework")]
extern "C" {
    fn CGDisplayCreateUUIDFromDisplayID(display: DisplayId) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFUUIDCreateString(alloc: *const c_void, uuid: *const c_void) -> *const c_void;
    fn CFStringGetCString(s: *const c_void, buf: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFRelease(cf: *const c_void);
}

/// `kCFStringEncodingUTF8`.
const UTF8: u32 = 0x0800_0100;
/// `kDisplayModeNativeFlag` (IOGraphicsTypes.h).
const NATIVE_MODE: u32 = 0x0200_0000;
/// `kCGDisplayBeginConfigurationFlag`: the callback before a change (ignored; the one after
/// counts).
const BEGIN_CONFIGURATION: u32 = 1;

/// A CFString as a Rust string.
fn cf_string(s: *const c_void) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let mut buf = [0 as c_char; 128];
    let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) };
    (ok != 0).then(|| {
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

/// The display's UUID as a string (`None` when CoreGraphics has none).
fn uuid(display: DisplayId) -> Option<String> {
    unsafe {
        let u = CGDisplayCreateUUIDFromDisplayID(display);
        if u.is_null() {
            return None;
        }
        let s = CFUUIDCreateString(std::ptr::null(), u);
        CFRelease(u);
        let out = cf_string(s);
        if !s.is_null() {
            CFRelease(s);
        }
        out
    }
}

/// EDID vendor letters (three 5-bit letters, as in the EDID header).
fn vendor_letters(v: u32) -> String {
    let letter = |shift: u32| {
        let c = ((v >> shift) & 0x1F) as u8;
        if (1..=26).contains(&c) {
            (b'A' + c - 1) as char
        } else {
            '?'
        }
    };
    [letter(10), letter(5), letter(0)].iter().collect()
}

/// `DEL-A0B1-12345678`, like the Windows backend's EDID key; empty when CoreGraphics knows no
/// vendor (virtual displays).
pub fn edid_key(vendor: u32, model: u32, serial: u32) -> String {
    if vendor == 0 || vendor == 0xFFFF_FFFF {
        return String::new();
    }
    format!(
        "{}-{:04X}-{serial:08X}",
        vendor_letters(vendor),
        model & 0xFFFF
    )
}

/// The current mode's backing pixels, refresh rate, and whether it is scaled (the native mode
/// has other pixels).
fn mode(display: DisplayId) -> (usize, usize, u32, bool) {
    unsafe {
        let m = CGDisplayCopyDisplayMode(display);
        if m.is_null() {
            return (0, 0, 60, false);
        }
        let (pw, ph) = (
            CGDisplayModeGetPixelWidth(m),
            CGDisplayModeGetPixelHeight(m),
        );
        let rate = CGDisplayModeGetRefreshRate(m).round() as u32;
        CGDisplayModeRelease(m);
        let mut native = 0usize;
        let all = CGDisplayCopyAllDisplayModes(display, std::ptr::null());
        if !all.is_null() {
            for i in 0..CFArrayGetCount(all) {
                let mm = CFArrayGetValueAtIndex(all, i);
                if CGDisplayModeGetIOFlags(mm) & NATIVE_MODE != 0 {
                    native = native.max(CGDisplayModeGetPixelWidth(mm));
                }
            }
            CFRelease(all);
        }
        let scaled = native != 0 && pw != 0 && pw != native;
        (pw, ph, if rate == 0 { 60 } else { rate }, scaled)
    }
}

/// NSScreen's localized names by display id (macOS 10.15+).
fn screen_names() -> Vec<(DisplayId, String)> {
    use objc2::msg_send;
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;
    let Some(cls) = AnyClass::get(c"NSScreen") else {
        return Vec::new();
    };
    autoreleasepool(|_| unsafe {
        let screens: Option<Retained<AnyObject>> = msg_send![cls, screens];
        let Some(screens) = screens else {
            return Vec::new();
        };
        let n: usize = msg_send![&*screens, count];
        let key = NSString::from_str("NSScreenNumber");
        let mut out = Vec::new();
        for i in 0..n {
            let s: Retained<AnyObject> = msg_send![&*screens, objectAtIndex: i];
            let desc: Option<Retained<AnyObject>> = msg_send![&*s, deviceDescription];
            let Some(desc) = desc else { continue };
            let num: Option<Retained<AnyObject>> = msg_send![&*desc, objectForKey: &*key];
            let Some(num) = num else { continue };
            let id: u32 = msg_send![&*num, unsignedIntValue];
            let name: Option<Retained<NSString>> = msg_send![&*s, localizedName];
            if let Some(name) = name {
                out.push((id, name.to_string()));
            }
        }
        out
    })
}

/// "Displays have separate Spaces" (System Settings › Desktop & Dock): a window can't span two
/// displays then.
fn separate_spaces() -> bool {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;
    let Some(cls) = AnyClass::get(c"NSScreen") else {
        return true;
    };
    unsafe { msg_send![cls, screensHaveSeparateSpaces] }
}

/// The main window's center in global points, if it exists (from the app).
pub type MainWindowCenter = Box<dyn Fn() -> Option<(f64, f64)> + Send + Sync>;

/// The system backend.
pub struct SystemBackend {
    main_window: MainWindowCenter,
}

impl SystemBackend {
    pub fn new(main_window: MainWindowCenter) -> Self {
        Self { main_window }
    }
}

impl DisplayBackend for SystemBackend {
    fn enumerate(&self) -> Vec<DisplayInfo> {
        enumerate((self.main_window)())
    }

    fn can_span(&self) -> bool {
        !separate_spaces()
    }

    fn watch(&self, changed: OnChange) -> std::io::Result<Box<dyn Watch>> {
        let thread = LoopThread::start(
            "displays",
            DEBOUNCE.as_secs_f64(),
            |waker| Box::new(Registration::new(waker)) as Box<dyn Any>,
            move |ev| {
                if ev == Event::Settled {
                    changed();
                }
                true
            },
        )?;
        Ok(Box::new(MacWatch(thread)))
    }
}

struct MacWatch(#[allow(dead_code)] LoopThread);

impl Watch for MacWatch {}

/// The reconfiguration callback, registered while the watcher runs (removed on its thread when
/// the loop ends).
struct Registration {
    user_info: *mut c_void,
}

extern "C" fn reconfigured(_display: DisplayId, flags: u32, user_info: *mut c_void) {
    if flags & BEGIN_CONFIGURATION != 0 || user_info.is_null() {
        return;
    }
    // SAFETY: `user_info` is the `Waker` the registration holds a reference to until it removes
    // this callback.
    let waker = unsafe { &*(user_info as *const Waker) };
    waker.changed();
}

impl Registration {
    fn new(waker: &Arc<Waker>) -> Self {
        let user_info = Arc::into_raw(waker.clone()) as *mut c_void;
        let r = unsafe { CGDisplayRegisterReconfigurationCallback(reconfigured, user_info) };
        if r != 0 {
            tracing::warn!("display hot-plug notifications unavailable (CGError {r})");
        }
        Self { user_info }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        unsafe {
            CGDisplayRemoveReconfigurationCallback(reconfigured, self.user_info);
            drop(Arc::from_raw(self.user_info as *const Waker));
        }
    }
}

/// The active displays now (a mirror set reports its main display only).
pub fn enumerate(main_window: Option<(f64, f64)>) -> Vec<DisplayInfo> {
    let mut ids = [0 as DisplayId; 32];
    let mut count = 0u32;
    if unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } != 0 {
        return Vec::new();
    }
    let names = screen_names();
    let mut out = Vec::new();
    for (n, &d) in ids[..count as usize].iter().enumerate() {
        if unsafe { CGDisplayMirrorsDisplay(d) } != 0 {
            continue;
        }
        let b = unsafe { CGDisplayBounds(d) };
        let (_, _, refresh_hz, scaled) = mode(d);
        let builtin = unsafe { CGDisplayIsBuiltin(d) } != 0;
        let name = names
            .iter()
            .find(|(id, _)| *id == d)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| {
                if builtin {
                    "Built-in display".to_string()
                } else {
                    format!("Display {}", n + 1)
                }
            });
        let key = unsafe {
            edid_key(
                CGDisplayVendorNumber(d),
                CGDisplayModelNumber(d),
                CGDisplaySerialNumber(d),
            )
        };
        let (x, y, w, h) = (b.origin.x, b.origin.y, b.size.width, b.size.height);
        out.push(DisplayInfo {
            id: uuid(d).unwrap_or_else(|| format!("cg-{d}")),
            name,
            edid_key: key,
            x: x.round() as i32,
            y: y.round() as i32,
            width: w.round().max(0.0) as u32,
            height: h.round().max(0.0) as u32,
            refresh_hz,
            primary: unsafe { CGDisplayIsMain(d) } != 0,
            has_main_window: main_window
                .is_some_and(|(cx, cy)| cx >= x && cx < x + w && cy >= y && cy < y + h),
            scaled,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edid_keys_match_the_windows_form() {
        // Dell's vendor id 0x10AC is "DEL".
        assert_eq!(edid_key(0x10AC, 0xA0B1, 0x1234_5678), "DEL-A0B1-12345678");
        assert_eq!(edid_key(0x0610, 0xA050, 0), "APP-A050-00000000");
        assert_eq!(edid_key(0, 1, 2), "");
        assert_eq!(edid_key(0xFFFF_FFFF, 1, 2), "");
    }

    /// The runner's (or the developer's) displays: at least one, one main, sane sizes, stable
    /// ids from one call to the next.
    #[test]
    fn enumerates_this_macs_displays() {
        let list = enumerate(None);
        println!("{list:#?}");
        assert!(!list.is_empty(), "no active display");
        assert_eq!(list.iter().filter(|d| d.primary).count(), 1);
        for d in &list {
            assert!(d.width >= 320 && d.height >= 200, "{d:?}");
            assert!(!d.id.is_empty() && !d.name.is_empty());
            assert!(!d.has_main_window);
        }
        let main = list.iter().find(|d| d.primary).unwrap();
        assert_eq!((main.x, main.y), (0, 0), "the main display is the origin");
        assert_eq!(enumerate(None), list);
        let center = (main.width as f64 / 2.0, main.height as f64 / 2.0);
        assert!(enumerate(Some(center))
            .iter()
            .any(|d| d.primary && d.has_main_window));
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGMainDisplayID() -> DisplayId;
        fn CGBeginDisplayConfiguration(config: *mut *mut c_void) -> i32;
        fn CGConfigureDisplayWithDisplayMode(
            config: *mut c_void,
            display: DisplayId,
            mode: *const c_void,
            options: *const c_void,
        ) -> i32;
        fn CGCompleteDisplayConfiguration(config: *mut c_void, option: u32) -> i32;
        fn CGDisplayModeIsUsableForDesktopGUI(mode: *const c_void) -> u8;
    }

    /// Switches the main display's mode for this process only (`kCGConfigureForAppOnly`: the
    /// system restores it when the process ends).
    fn configure(display: DisplayId, mode: *const c_void) {
        unsafe {
            let mut cfg = std::ptr::null_mut();
            assert_eq!(CGBeginDisplayConfiguration(&mut cfg), 0);
            assert_eq!(
                CGConfigureDisplayWithDisplayMode(cfg, display, mode, std::ptr::null()),
                0
            );
            assert_eq!(CGCompleteDisplayConfiguration(cfg, 0), 0);
        }
    }

    /// A real display change (another mode of the main display, then back) reaches the watcher,
    /// debounced. Changes the display briefly, so not in a normal run; CI runs it
    /// (`.github/workflows/macos-spike.yml`).
    #[test]
    #[ignore = "switches the main display's mode for a moment"]
    fn a_display_change_reaches_the_watcher() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        let backend = SystemBackend::new(Box::new(|| None));
        let (tx, rx) = mpsc::channel();
        let watch = backend
            .watch(Box::new(move || {
                let _ = tx.send(());
            }))
            .unwrap();
        unsafe {
            let main = CGMainDisplayID();
            let current = CGDisplayCopyDisplayMode(main);
            let width = CGDisplayModeGetPixelWidth(current);
            let all = CGDisplayCopyAllDisplayModes(main, std::ptr::null());
            let other = (0..CFArrayGetCount(all))
                .map(|i| CFArrayGetValueAtIndex(all, i))
                .find(|&m| {
                    CGDisplayModeIsUsableForDesktopGUI(m) != 0
                        && CGDisplayModeGetPixelWidth(m) != width
                });
            let Some(other) = other else {
                println!("only one display mode here: nothing to switch to");
                CFRelease(all);
                CGDisplayModeRelease(current);
                return;
            };
            let before = enumerate(None);
            let t0 = Instant::now();
            configure(main, other);
            rx.recv_timeout(Duration::from_secs(5))
                .expect("the change reached the watcher");
            let took = t0.elapsed();
            println!("mode change → watcher: {took:?}");
            assert!(took >= Duration::from_millis(250), "debounced: {took:?}");
            assert_ne!(enumerate(None), before, "the list changed");
            configure(main, current);
            rx.recv_timeout(Duration::from_secs(5))
                .expect("the change back reached the watcher");
            assert_eq!(enumerate(None), before);
            CFRelease(all);
            CGDisplayModeRelease(current);
        }
        drop(watch);
    }
}
