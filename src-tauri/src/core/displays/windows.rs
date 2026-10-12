//! The Windows display backend: enumeration (GDI + the EDID in the registry) and the `displays`
//! watcher thread (a hidden top-level window: message-only windows don't receive the
//! `WM_DISPLAYCHANGE` / `WM_SETTINGCHANGE` broadcasts).

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc;
use std::thread::{JoinHandle, ThreadId};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplaySettingsExW, GetMonitorInfoW, MonitorFromWindow, DEVMODEW,
    DISPLAY_DEVICEW, DISPLAY_DEVICE_ACTIVE, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
    DISPLAY_DEVICE_MIRRORING_DRIVER, DISPLAY_DEVICE_PRIMARY_DEVICE, ENUM_CURRENT_SETTINGS,
    ENUM_DISPLAY_SETTINGS_FLAGS, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONULL,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer,
    PostMessageW, RegisterClassExW, SetTimer, EDD_GET_DEVICE_INTERFACE_NAME, MSG, WINDOW_EX_STYLE,
    WM_APP, WM_DEVICECHANGE, WM_DISPLAYCHANGE, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

use super::{parse_edid, DisplayBackend, OnChange, Watch, DEBOUNCE};
use crate::core::model::DisplayInfo;

/// End the thread.
const MSG_QUIT: u32 = WM_APP;
const TIMER_DISPLAYS: usize = 1;
/// `wParam` of WM_DEVICECHANGE (Dbt.h).
const DBT_DEVNODES_CHANGED: usize = 0x0007;
/// `wParam` of WM_SETTINGCHANGE when the work area changed (SPI_SETWORKAREA).
const SPI_SETWORKAREA: usize = 0x002F;

fn wide_str(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The system backend.
pub struct SystemBackend {
    main_hwnd: AtomicIsize,
}

impl SystemBackend {
    pub fn new(main_hwnd: isize) -> Self {
        Self {
            main_hwnd: AtomicIsize::new(main_hwnd),
        }
    }
}

impl DisplayBackend for SystemBackend {
    fn enumerate(&self) -> Vec<DisplayInfo> {
        enumerate(self.main_hwnd.load(Ordering::Relaxed))
    }

    fn watch(&self, changed: OnChange) -> std::io::Result<Box<dyn Watch>> {
        Service::start(changed).map(|s| Box::new(s) as Box<dyn Watch>)
    }
}

/// The GDI device name (`\\.\DISPLAY1`) of the monitor the main window is on.
fn main_monitor_device(hwnd: isize) -> Option<String> {
    if hwnd == 0 {
        return None;
    }
    // SAFETY: plain queries; a stale window handle makes them fail (null monitor).
    unsafe {
        let mon = MonitorFromWindow(HWND(hwnd as *mut _), MONITOR_DEFAULTTONULL);
        if mon.is_invalid() {
            return None;
        }
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        GetMonitorInfoW(mon, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO)
            .as_bool()
            .then(|| wide_str(&info.szDevice))
    }
}

/// The monitor's EDID bytes from the registry.
fn read_edid(subkey: &str) -> Option<Vec<u8>> {
    let key = wide(subkey);
    let mut buf = vec![0u8; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: the buffer and its length are valid for the call.
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            w!("EDID"),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            Some(&mut len),
        )
    };
    if r.0 != 0 {
        return None;
    }
    buf.truncate(len as usize);
    Some(buf)
}

/// Every display attached to the desktop, in adapter order.
pub fn enumerate(main_hwnd: isize) -> Vec<DisplayInfo> {
    let main_dev = main_monitor_device(main_hwnd);
    let mut out = Vec::new();
    // SAFETY: the structs are sized (`cb` / `dmSize`) and live across each call.
    unsafe {
        for i in 0..64u32 {
            let mut adapter = DISPLAY_DEVICEW {
                cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                ..Default::default()
            };
            if !EnumDisplayDevicesW(PCWSTR::null(), i, &mut adapter, 0).as_bool() {
                break;
            }
            let flags = adapter.StateFlags;
            if flags.0 & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP.0 == 0
                || flags.0 & DISPLAY_DEVICE_MIRRORING_DRIVER.0 != 0
            {
                continue;
            }
            let mut dm = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            if !EnumDisplaySettingsExW(
                PCWSTR(adapter.DeviceName.as_ptr()),
                ENUM_CURRENT_SETTINGS,
                &mut dm,
                ENUM_DISPLAY_SETTINGS_FLAGS(0),
            )
            .as_bool()
            {
                continue;
            }
            let pos = dm.Anonymous1.Anonymous2.dmPosition;
            let device = wide_str(&adapter.DeviceName);
            // The monitor on this adapter output (the first active one; clones share a source).
            let mut monitor = None;
            for j in 0..16u32 {
                let mut m = DISPLAY_DEVICEW {
                    cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                    ..Default::default()
                };
                if !EnumDisplayDevicesW(
                    PCWSTR(adapter.DeviceName.as_ptr()),
                    j,
                    &mut m,
                    EDD_GET_DEVICE_INTERFACE_NAME,
                )
                .as_bool()
                {
                    break;
                }
                if m.StateFlags.0 & DISPLAY_DEVICE_ACTIVE.0 != 0 {
                    monitor = Some(m);
                    break;
                }
            }
            let (id, monitor_desc) = match &monitor {
                Some(m) if m.DeviceID[0] != 0 => (wide_str(&m.DeviceID), wide_str(&m.DeviceString)),
                _ => (device.clone(), String::new()),
            };
            let edid = super::edid_registry_key(&id)
                .and_then(|k| read_edid(&k))
                .and_then(|b| parse_edid(&b));
            let name = edid
                .as_ref()
                .and_then(|e| e.name.clone())
                .or_else(|| {
                    let d = monitor_desc.trim();
                    (!d.is_empty() && !d.eq_ignore_ascii_case("Generic PnP Monitor"))
                        .then(|| d.to_string())
                })
                .unwrap_or_else(|| wide_str(&adapter.DeviceString).trim().to_string());
            out.push(DisplayInfo {
                id,
                name,
                edid_key: edid.map(|e| e.key).unwrap_or_default(),
                x: pos.x,
                y: pos.y,
                width: dm.dmPelsWidth,
                height: dm.dmPelsHeight,
                refresh_hz: dm.dmDisplayFrequency,
                primary: flags.0 & DISPLAY_DEVICE_PRIMARY_DEVICE.0 != 0,
                has_main_window: main_dev.as_deref() == Some(device.as_str()),
            });
        }
    }
    out
}

// ---- Watcher thread ---------------------------------------------------------------------------

/// The `displays` thread. Dropping it from another thread stops and joins it.
struct Service {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
    id: ThreadId,
}

impl Watch for Service {}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let poke = match msg {
        WM_DISPLAYCHANGE => true,
        WM_DEVICECHANGE => wp.0 == DBT_DEVNODES_CHANGED,
        WM_SETTINGCHANGE => wp.0 == SPI_SETWORKAREA,
        _ => false,
    };
    if poke {
        // (Re)starts the debounce timer; WM_TIMER is posted to the loop below.
        SetTimer(
            Some(hwnd),
            TIMER_DISPLAYS,
            DEBOUNCE.as_millis() as u32,
            None,
        );
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

impl Service {
    fn start(changed: OnChange) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Result<isize, String>>(1);
        let thread = std::thread::Builder::new()
            .name("displays".into())
            .spawn(move || run(tx, changed))?;
        let id = thread.thread().id();
        match rx.recv() {
            Ok(Ok(hwnd)) => Ok(Service {
                hwnd,
                thread: Some(thread),
                id,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(std::io::Error::other(e))
            }
            Err(_) => Err(std::io::Error::other("displays thread ended")),
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        // SAFETY: PostMessageW only queues; a stale handle fails harmlessly.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.hwnd as *mut core::ffi::c_void)),
                MSG_QUIT,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if std::thread::current().id() == self.id {
            return; // the loop ends after the running callback
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(ready: mpsc::SyncSender<Result<isize, String>>, changed: OnChange) {
    // SAFETY: plain Win32 window setup on this thread; the window is used on this thread only
    // (PostMessageW from others is allowed) and destroyed before it returns.
    unsafe {
        let module = match GetModuleHandleW(None) {
            Ok(m) => m,
            Err(e) => {
                let _ = ready.send(Err(e.to_string()));
                return;
            }
        };
        let instance = HINSTANCE(module.0);
        let class = w!("KinetraDisplaysService");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class,
            ..Default::default()
        };
        // Fails harmlessly when the class exists (a previous thread registered it).
        RegisterClassExW(&wc);
        // A hidden top-level window (never shown): receives the display broadcasts.
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0),
            class,
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                let _ = ready.send(Err(e.to_string()));
                return;
            }
        };
        let _ = ready.send(Ok(hwnd.0 as isize));
        drop(ready);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            match msg.message {
                MSG_QUIT => break,
                WM_TIMER if msg.wParam.0 == TIMER_DISPLAYS => {
                    let _ = KillTimer(Some(hwnd), TIMER_DISPLAYS);
                    changed();
                }
                _ => {
                    DispatchMessageW(&msg);
                }
            }
        }
        let _ = DestroyWindow(hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Enumerates the real displays (no window is created). Skips when the machine has none
    /// (a headless CI session).
    #[test]
    fn lists_the_connected_displays() {
        let list = enumerate(0);
        if list.is_empty() {
            eprintln!("no displays attached; skipped");
            return;
        }
        assert!(list.iter().filter(|d| d.primary).count() <= 1);
        for d in &list {
            assert!(!d.id.is_empty(), "{d:?}");
            assert!(d.width > 0 && d.height > 0, "{d:?}");
            assert!(!d.has_main_window);
            assert!(d.edid_key.is_empty() || d.edid_key.len() >= 8, "{d:?}");
        }
        let mut ids: Vec<_> = list.iter().map(|d| &d.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), list.len(), "ids are unique");
        eprintln!("{list:#?}");
    }
}
