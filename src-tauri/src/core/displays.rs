//! Connected displays: enumeration, EDID names, hot-plug, `displays-changed`.
//!
//! # Backends
//! A [`DisplayBackend`] enumerates and watches. The macOS backend ([`macos`]) uses CoreGraphics
//! and a run-loop watcher thread (points, not pixels: see there). The Windows backend ([`windows`]) uses
//! `EnumDisplayDevicesW` (adapters → their monitor, `EDD_GET_DEVICE_INTERFACE_NAME` for the
//! stable id), `EnumDisplaySettingsExW(ENUM_CURRENT_SETTINGS)` (position, size, refresh rate)
//! and the monitor's EDID from the registry (name, manufacturer / product / serial). Tests use
//! [`TestBackend`]. Without a backend (the default in tests) there are no displays.
//!
//! # Watching
//! The watcher (system backend: ONE `displays` thread with a hidden top-level window; message-
//! only windows don't get `WM_DISPLAYCHANGE`) exists only while it is needed: at least one
//! output is on (enabled or showing a test pattern) or a `watch_displays` session is open
//! (counted; Preferences › Outputs). It blocks in `GetMessageW`; `WM_DISPLAYCHANGE`,
//! `WM_DEVICECHANGE` (`DBT_DEVNODES_CHANGED`) and `WM_SETTINGCHANGE` (work area) (re)start a
//! 300 ms timer; when it fires the displays are enumerated again and diffed against the last
//! list. Only a real change emits `displays-changed` and reconciles the outputs. No polling.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::thread::{JoinHandle, ThreadId};
use std::time::Duration;

use crate::core::events::names;
use crate::core::model::DisplayInfo;
use crate::core::state::AppState;

/// Quiet time after the last OS notification before the displays are enumerated again (a
/// plug-in sends several).
pub const DEBOUNCE: Duration = Duration::from_millis(300);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Called on the watcher's thread after each debounced OS notification.
pub type OnChange = Box<dyn Fn() + Send + Sync>;

/// A running watcher. Dropping it stops it (and joins its thread, unless dropped on that
/// thread).
pub trait Watch: Send {}

pub trait DisplayBackend: Send + Sync {
    /// The connected displays, now.
    fn enumerate(&self) -> Vec<DisplayInfo>;
    /// Starts watching; `changed` runs after each debounced notification.
    fn watch(&self, changed: OnChange) -> std::io::Result<Box<dyn Watch>>;
    /// One window may span two adjacent displays (a frame-locked key + fill pair). macOS: only
    /// while "Displays have separate Spaces" is off.
    fn can_span(&self) -> bool {
        true
    }
}

// ---- EDID -------------------------------------------------------------------------------------

/// What Kinetra reads from a monitor's EDID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edid {
    /// The monitor name descriptor (0xFC), trimmed; `None` when absent.
    pub name: Option<String>,
    /// Manufacturer + product + serial (`DEL-A0B1-12345678`; the serial string descriptor
    /// (0xFF) replaces a zero serial number).
    pub key: String,
}

/// Parses an EDID base block (≥ 128 bytes, the fixed header). `None` when it isn't one.
pub fn parse_edid(b: &[u8]) -> Option<Edid> {
    const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    if b.len() < 128 || b[..8] != HEADER {
        return None;
    }
    let v = u16::from_be_bytes([b[8], b[9]]);
    let letter = |shift: u16| {
        let c = ((v >> shift) & 0x1F) as u8;
        if (1..=26).contains(&c) {
            (b'A' + c - 1) as char
        } else {
            '?'
        }
    };
    let mfg: String = [letter(10), letter(5), letter(0)].iter().collect();
    let product = u16::from_le_bytes([b[10], b[11]]);
    let serial = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
    let mut name = None;
    let mut serial_text = None;
    for at in [54usize, 72, 90, 108] {
        let d = &b[at..at + 18];
        if d[0] != 0 || d[1] != 0 || d[2] != 0 {
            continue; // a detailed timing descriptor
        }
        let text = || descriptor_text(&d[5..18]);
        match d[3] {
            0xFC => name = text(),
            0xFF => serial_text = text(),
            _ => {}
        }
    }
    let key = match (serial, serial_text) {
        (0, Some(s)) => format!("{mfg}-{product:04X}-{s}"),
        _ => format!("{mfg}-{product:04X}-{serial:08X}"),
    };
    Some(Edid { name, key })
}

/// A descriptor's text: ASCII up to the 0x0A terminator, trimmed; `None` when empty.
fn descriptor_text(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|&c| c == 0x0A).unwrap_or(bytes.len());
    let s: String = bytes[..end]
        .iter()
        .filter(|c| c.is_ascii_graphic() || **c == b' ')
        .map(|&c| c as char)
        .collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// The registry key holding a monitor's EDID (under `HKLM`), from its device interface path:
/// `\\?\DISPLAY#DEL4123#5&2a3b&0&UID4353#{e6f07b5f-…}` →
/// `SYSTEM\CurrentControlSet\Enum\DISPLAY\DEL4123\5&2a3b&0&UID4353\Device Parameters`.
pub fn edid_registry_key(interface: &str) -> Option<String> {
    let rest = interface.strip_prefix(r"\\?\")?;
    let mut parts = rest.split('#');
    let class = parts.next()?;
    let hwid = parts.next()?;
    let instance = parts.next()?;
    let ok = |s: &str| !s.is_empty() && !s.contains('\\') && !s.contains('/');
    if !class.eq_ignore_ascii_case("DISPLAY") || !ok(hwid) || !ok(instance) {
        return None;
    }
    Some(format!(
        r"SYSTEM\CurrentControlSet\Enum\DISPLAY\{hwid}\{instance}\Device Parameters"
    ))
}

// ---- Runtime ----------------------------------------------------------------------------------

/// Display runtime (one per `AppState`).
#[derive(Default)]
pub struct Displays {
    backend: RwLock<Option<Arc<dyn DisplayBackend>>>,
    /// `watch_displays` sessions.
    watchers: AtomicUsize,
    /// The outputs need the watcher (an output is on).
    needed: AtomicBool,
    /// The running watcher.
    svc: Mutex<Option<Box<dyn Watch>>>,
    /// The list last enumerated (`None`: never).
    list: Mutex<Option<Vec<DisplayInfo>>>,
    /// Debounced OS notifications handled (diagnostics, tests).
    os_changes: AtomicU64,
}

impl Displays {
    fn backend(&self) -> Option<Arc<dyn DisplayBackend>> {
        self.backend
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether the watcher runs.
    pub fn watching(&self) -> bool {
        lock(&self.svc).is_some()
    }

    /// Debounced OS notifications handled so far.
    pub fn os_changes(&self) -> u64 {
        self.os_changes.load(Ordering::Relaxed)
    }
}

impl AppState {
    /// The Windows backend. `main_hwnd` is the main window (for `hasMainWindow`; 0 = none).
    #[cfg(windows)]
    pub fn use_system_displays(&self, main_hwnd: isize) {
        self.set_display_backend(Some(Arc::new(windows::SystemBackend::new(main_hwnd))));
    }

    /// The macOS backend. `main_window` gives the main window's center in global points.
    #[cfg(target_os = "macos")]
    pub fn use_system_displays(&self, main_window: macos::MainWindowCenter) {
        self.set_display_backend(Some(Arc::new(macos::SystemBackend::new(main_window))));
    }

    /// Replaces the display backend (`None`: no displays). Stops the old watcher.
    pub fn set_display_backend(&self, backend: Option<Arc<dyn DisplayBackend>>) {
        let d = self.displays_rt();
        drop(lock(&d.svc).take());
        *d.backend.write().unwrap_or_else(PoisonError::into_inner) = backend;
        *lock(&d.list) = None;
        self.displays_service_reconcile();
        self.outputs_request();
    }

    /// Enumerates the displays now (and announces a change).
    pub fn list_displays(&self) -> Vec<DisplayInfo> {
        self.displays_refresh(true)
    }

    /// Enumerates and stores the list; a change emits `displays-changed` and (with
    /// `reconcile`) reconciles the outputs.
    pub(crate) fn displays_refresh(&self, reconcile: bool) -> Vec<DisplayInfo> {
        let d = self.displays_rt();
        let list = d.backend().map(|b| b.enumerate()).unwrap_or_default();
        let changed = {
            let mut g = lock(&d.list);
            let changed = g.as_ref() != Some(&list);
            *g = Some(list.clone());
            changed
        };
        if changed {
            match serde_json::to_value(&list) {
                Ok(v) => self.events().emit(names::DISPLAYS_CHANGED, v),
                Err(e) => tracing::warn!("displays-changed not sent: {e}"),
            }
            if reconcile {
                self.outputs_request();
            }
        }
        list
    }

    /// The displays as last seen while watched; enumerated now when nothing watches them (the
    /// list could be stale).
    pub(crate) fn displays_current(&self) -> Vec<DisplayInfo> {
        let d = self.displays_rt();
        if d.watching() {
            if let Some(l) = lock(&d.list).clone() {
                return l;
            }
        }
        self.displays_refresh(false)
    }

    /// Whether one window may span two displays now ([`DisplayBackend::can_span`]).
    pub(crate) fn displays_can_span(&self) -> bool {
        self.displays_rt().backend().is_none_or(|b| b.can_span())
    }

    /// The last enumerated list, without enumerating (empty when never).
    pub(crate) fn displays_cached(&self) -> Vec<DisplayInfo> {
        lock(&self.displays_rt().list).clone().unwrap_or_default()
    }

    /// Opens (`on`) or closes a `watch_displays` session; returns the displays now.
    pub fn watch_displays(&self, on: bool) -> Vec<DisplayInfo> {
        let d = self.displays_rt();
        if on {
            d.watchers.fetch_add(1, Ordering::SeqCst);
        } else {
            let _ = d
                .watchers
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
        }
        self.displays_service_reconcile();
        self.list_displays()
    }

    /// Whether the outputs need the watcher (called by the outputs' reconcile with no lock
    /// held).
    pub(crate) fn displays_set_needed(&self, needed: bool) {
        let d = self.displays_rt();
        if d.needed.swap(needed, Ordering::SeqCst) != needed {
            self.displays_service_reconcile();
        }
    }

    /// Starts or stops the watcher so it runs exactly while needed.
    fn displays_service_reconcile(&self) {
        let d = self.displays_rt();
        let want = d.watchers.load(Ordering::SeqCst) > 0 || d.needed.load(Ordering::SeqCst);
        let stopped = {
            let mut svc = lock(&d.svc);
            match (want, svc.is_some()) {
                (true, false) => {
                    let Some(backend) = d.backend() else {
                        return;
                    };
                    let weak = self.downgrade();
                    match backend.watch(Box::new(move || {
                        if let Some(st) = weak.upgrade() {
                            st.displays_rt().os_changes.fetch_add(1, Ordering::Relaxed);
                            st.displays_refresh(true);
                        }
                    })) {
                        Ok(w) => *svc = Some(w),
                        Err(e) => tracing::warn!("display hot-plug watcher not started: {e}"),
                    }
                    None
                }
                (false, true) => svc.take(),
                _ => None,
            }
        };
        // Joined outside the lock (its callback may be waiting for it).
        drop(stopped);
    }

    /// Stops the watcher (app exit).
    pub fn displays_shutdown(&self) {
        let d = self.displays_rt();
        d.watchers.store(0, Ordering::SeqCst);
        d.needed.store(false, Ordering::SeqCst);
        let svc = lock(&d.svc).take();
        drop(svc);
    }
}

// ---- Test backend -----------------------------------------------------------------------------

/// In-memory displays for tests and the e2e fixture: a settable list and a watcher thread that
/// debounces simulated OS notifications like the system one ([`TestBackend::notify`]).
pub struct TestBackend {
    list: Mutex<Vec<DisplayInfo>>,
    debounce: Duration,
    tx: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    enumerations: AtomicUsize,
    threads: Arc<AtomicUsize>,
}

impl TestBackend {
    pub fn new(list: Vec<DisplayInfo>, debounce: Duration) -> Self {
        Self {
            list: Mutex::new(list),
            debounce,
            tx: Arc::new(Mutex::new(None)),
            enumerations: AtomicUsize::new(0),
            threads: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Replaces the connected displays (no notification: see [`TestBackend::notify`]).
    pub fn set(&self, list: Vec<DisplayInfo>) {
        *lock(&self.list) = list;
    }

    /// Simulates one OS notification (`WM_DISPLAYCHANGE`). Ignored while not watching.
    pub fn notify(&self) {
        if let Some(tx) = lock(&self.tx).as_ref() {
            let _ = tx.send(());
        }
    }

    pub fn enumerations(&self) -> usize {
        self.enumerations.load(Ordering::SeqCst)
    }

    /// Watcher threads alive now.
    pub fn threads(&self) -> usize {
        self.threads.load(Ordering::SeqCst)
    }
}

struct TestWatch {
    tx: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    thread: Option<JoinHandle<()>>,
    id: ThreadId,
}

impl Watch for TestWatch {}

impl Drop for TestWatch {
    fn drop(&mut self) {
        // Disconnecting the channel ends the thread.
        lock(&self.tx).take();
        if std::thread::current().id() != self.id {
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}

impl DisplayBackend for TestBackend {
    fn enumerate(&self) -> Vec<DisplayInfo> {
        self.enumerations.fetch_add(1, Ordering::SeqCst);
        lock(&self.list).clone()
    }

    fn watch(&self, changed: OnChange) -> std::io::Result<Box<dyn Watch>> {
        let (tx, rx) = mpsc::channel::<()>();
        *lock(&self.tx) = Some(tx);
        let debounce = self.debounce;
        let threads = self.threads.clone();
        threads.fetch_add(1, Ordering::SeqCst);
        let thread = std::thread::Builder::new()
            .name("displays-test".into())
            .spawn(move || {
                'outer: while rx.recv().is_ok() {
                    loop {
                        match rx.recv_timeout(debounce) {
                            Ok(()) => continue,
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => break 'outer,
                        }
                    }
                    changed();
                }
                threads.fetch_sub(1, Ordering::SeqCst);
            })?;
        let id = thread.thread().id();
        Ok(Box::new(TestWatch {
            tx: self.tx.clone(),
            thread: Some(thread),
            id,
        }))
    }
}

#[cfg(test)]
mod tests;
