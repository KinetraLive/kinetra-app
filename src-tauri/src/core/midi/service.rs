//! The MIDI service thread (Windows): one message-only window that
//! - receives device notifications (`RegisterDeviceNotificationW` with every device interface
//!   class: USB MIDI devices arrive as audio / MIDI interfaces). A notification starts (or
//!   restarts) a 300 ms window timer; when it fires the ports are re-enumerated (debounce: a
//!   plug-in sends several notifications);
//! - is woken by `PostMessageW` from WinMM's input callback (input queued) and by reconcile
//!   requests from other threads.
//!
//! It blocks in `GetMessageW` between messages: no polling, 0 % CPU at rest. It exists only while
//! MIDI is in use (see `core::midi`).

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{JoinHandle, ThreadId};

use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer,
    PostMessageW, RegisterClassExW, RegisterDeviceNotificationW, SetTimer,
    UnregisterDeviceNotification, DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_ALL_INTERFACE_CLASSES,
    DEVICE_NOTIFY_WINDOW_HANDLE, DEV_BROADCAST_DEVICEINTERFACE_W, HWND_MESSAGE, MSG,
    REGISTER_NOTIFICATION_FLAGS, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_DEVICECHANGE, WM_TIMER,
    WNDCLASSEXW,
};

use super::InputQueue;

/// Input was queued.
const MSG_INPUT: u32 = WM_APP;
/// Re-evaluate the ports.
const MSG_RECONCILE: u32 = WM_APP + 1;
/// End the thread.
const MSG_QUIT: u32 = WM_APP + 2;
/// The hot-plug debounce timer.
const TIMER_DEVICES: usize = 1;
const DEBOUNCE_MS: u32 = 300;

// `wParam` values of WM_DEVICECHANGE (Dbt.h).
const DBT_DEVNODES_CHANGED: usize = 0x0007;
const DBT_DEVICEARRIVAL: usize = 0x8000;
const DBT_DEVICEREMOVECOMPLETE: usize = 0x8004;

/// What the service thread asks its handler to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Run the queued input.
    Input,
    /// Re-evaluate the ports (a request, or devices changed).
    Reconcile,
}

/// The running thread. Dropping it from another thread stops and joins it; dropped on its own
/// thread (the handler said to end) it only detaches.
pub struct Service {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
    id: ThreadId,
    queue: Arc<InputQueue>,
}

fn post(hwnd: isize, msg: u32) {
    if hwnd == 0 {
        return;
    }
    // SAFETY: PostMessageW only queues a message; a stale handle makes it fail harmlessly.
    let r = unsafe {
        PostMessageW(
            Some(HWND(hwnd as *mut core::ffi::c_void)),
            msg,
            WPARAM(0),
            LPARAM(0),
        )
    };
    if let Err(e) = r {
        tracing::debug!("MIDI service post: {e}");
    }
}

/// Wakes the service for queued input (called on WinMM's callback thread).
pub fn post_input(hwnd: isize) {
    post(hwnd, MSG_INPUT);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_DEVICECHANGE {
        if matches!(
            wp.0,
            DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE | DBT_DEVNODES_CHANGED
        ) {
            // (Re)starts the debounce timer; WM_TIMER is posted to the loop below.
            SetTimer(Some(hwnd), TIMER_DEVICES, DEBOUNCE_MS, None);
        }
        return LRESULT(1);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

impl Service {
    /// Starts the thread; `handler` runs on it for every event and returns false to end it.
    pub fn start(
        queue: Arc<InputQueue>,
        handler: impl Fn(Event) -> bool + Send + 'static,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Result<isize, String>>(1);
        let q = queue.clone();
        let thread = std::thread::Builder::new()
            .name("midi".into())
            .spawn(move || run(q, tx, handler))?;
        let id = thread.thread().id();
        match rx.recv() {
            Ok(Ok(hwnd)) => Ok(Service {
                hwnd,
                thread: Some(thread),
                id,
                queue,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(std::io::Error::other(e))
            }
            Err(_) => Err(std::io::Error::other("MIDI service thread ended")),
        }
    }

    /// Asks the thread to re-evaluate the ports.
    pub fn reconcile(&self) {
        post(self.hwnd, MSG_RECONCILE);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if std::thread::current().id() == self.id {
            // The thread ends by itself after its handler returns.
            return;
        }
        post(self.hwnd, MSG_QUIT);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.queue.hwnd.store(0, Ordering::Release);
    }
}

fn run(
    queue: Arc<InputQueue>,
    ready: mpsc::SyncSender<Result<isize, String>>,
    handler: impl Fn(Event) -> bool,
) {
    // SAFETY: plain Win32 window setup on this thread; every handle is used on this thread only
    // (PostMessageW from others is allowed) and released before it returns.
    unsafe {
        let module = match GetModuleHandleW(None) {
            Ok(m) => m,
            Err(e) => {
                let _ = ready.send(Err(e.to_string()));
                return;
            }
        };
        let instance = HINSTANCE(module.0);
        let class = w!("KinetraMidiService");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class,
            ..Default::default()
        };
        // Fails harmlessly when the class exists (a previous service thread registered it).
        RegisterClassExW(&wc);
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
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
        let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            ..Default::default()
        };
        let notify = RegisterDeviceNotificationW(
            HANDLE(hwnd.0),
            &filter as *const _ as *const core::ffi::c_void,
            REGISTER_NOTIFICATION_FLAGS(
                DEVICE_NOTIFY_WINDOW_HANDLE.0 | DEVICE_NOTIFY_ALL_INTERFACE_CLASSES.0,
            ),
        );
        if let Err(e) = &notify {
            tracing::warn!("MIDI hot-plug notifications unavailable: {e}");
        }
        let raw = hwnd.0 as isize;
        queue.hwnd.store(raw, Ordering::Release);
        let _ = ready.send(Ok(raw));
        drop(ready);

        // The first reconcile opens the ports; input queued before the window existed runs too.
        let mut alive = handler(Event::Reconcile) && handler(Event::Input);
        let mut msg = MSG::default();
        while alive && GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            alive = match msg.message {
                MSG_INPUT => handler(Event::Input),
                MSG_RECONCILE => handler(Event::Reconcile),
                MSG_QUIT => false,
                WM_TIMER if msg.wParam.0 == TIMER_DEVICES => {
                    let _ = KillTimer(Some(hwnd), TIMER_DEVICES);
                    handler(Event::Reconcile)
                }
                _ => {
                    DispatchMessageW(&msg);
                    true
                }
            };
        }
        queue.hwnd.store(0, Ordering::Release);
        if let Ok(n) = notify {
            let _ = UnregisterDeviceNotification(n);
        }
        let _ = DestroyWindow(hwnd);
    }
}
