//! The MIDI service thread (macOS): one thread running a CoreFoundation run loop that
//! - holds a CoreMIDI client with notifications (CoreMIDI delivers them on the run loop current
//!   when the client was made, this one). A setup change (a device or port added or removed)
//!   starts (or restarts) a 300 ms run-loop timer; when it fires the ports are re-enumerated
//!   (debounce: a plug-in sends several notifications);
//! - is woken through a run-loop source by CoreMIDI's input thread (input queued) and by
//!   reconcile requests from other threads.
//!
//! It blocks in `CFRunLoopRun` between events: no polling, 0 % CPU at rest. It exists only while
//! MIDI is in use (see `core::midi`). The same shape as the Windows service (`service.rs`).
//!
//! CoreMIDI clients are never disposed (disposing an app's last client can stop the MIDI server,
//! after which no client can be made): one notification client per service start, which happens
//! when MIDI comes into use.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc, Arc, PoisonError, RwLock};
use std::thread::{JoinHandle, ThreadId};

use super::InputQueue;

/// The hot-plug debounce.
const DEBOUNCE_S: f64 = 0.3;

const INPUT: u8 = 1;
const RECONCILE: u8 = 2;
const DEVICES: u8 = 4;
const QUIT: u8 = 8;

/// What the service thread asks its handler to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Run the queued input.
    Input,
    /// Re-evaluate the ports (a request, or devices changed).
    Reconcile,
}

// ---- CoreFoundation (declared by hand: a few run-loop functions) ----

type CFRunLoopRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopTimerRef = *mut c_void;
type CFStringRef = *const c_void;
type CFIndex = isize;

#[repr(C)]
struct CFRunLoopSourceContext {
    version: CFIndex,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    copy_description: Option<extern "C" fn(*const c_void) -> CFStringRef>,
    equal: Option<extern "C" fn(*const c_void, *const c_void) -> u8>,
    hash: Option<extern "C" fn(*const c_void) -> usize>,
    schedule: Option<extern "C" fn(*mut c_void, CFRunLoopRef, CFStringRef)>,
    cancel: Option<extern "C" fn(*mut c_void, CFRunLoopRef, CFStringRef)>,
    perform: Option<extern "C" fn(*mut c_void)>,
}

#[repr(C)]
struct CFRunLoopTimerContext {
    version: CFIndex,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    copy_description: Option<extern "C" fn(*const c_void) -> CFStringRef>,
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: CFStringRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: CFRunLoopRef);
    fn CFRunLoopWakeUp(rl: CFRunLoopRef);
    fn CFRunLoopSourceCreate(
        alloc: *const c_void,
        order: CFIndex,
        context: *mut CFRunLoopSourceContext,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopSourceSignal(source: CFRunLoopSourceRef);
    fn CFRunLoopSourceInvalidate(source: CFRunLoopSourceRef);
    fn CFRunLoopTimerCreate(
        alloc: *const c_void,
        fire_date: f64,
        interval: f64,
        flags: u64,
        order: CFIndex,
        callout: extern "C" fn(CFRunLoopTimerRef, *mut c_void),
        context: *mut CFRunLoopTimerContext,
    ) -> CFRunLoopTimerRef;
    fn CFRunLoopAddTimer(rl: CFRunLoopRef, timer: CFRunLoopTimerRef, mode: CFStringRef);
    fn CFRunLoopTimerSetNextFireDate(timer: CFRunLoopTimerRef, date: f64);
    fn CFRunLoopTimerInvalidate(timer: CFRunLoopTimerRef);
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFRetain(cf: *const c_void) -> *const c_void;
    fn CFRelease(cf: *const c_void);
}

/// Wakes the service's run loop with a reason. Shared with the input queue (CoreMIDI's input
/// thread) and the [`Service`] handle; holds its own references to the run loop and the source,
/// so signalling after the thread ended is harmless.
pub struct Waker {
    flags: AtomicU8,
    run_loop: CFRunLoopRef,
    source: CFRunLoopSourceRef,
}

// SAFETY: CFRunLoopSourceSignal and CFRunLoopWakeUp may be called from any thread; both objects
// are retained until the waker is dropped.
unsafe impl Send for Waker {}
unsafe impl Sync for Waker {}

impl Waker {
    fn signal(&self, reason: u8) {
        self.flags.fetch_or(reason, Ordering::AcqRel);
        unsafe {
            CFRunLoopSourceSignal(self.source);
            CFRunLoopWakeUp(self.run_loop);
        }
    }
}

impl Drop for Waker {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.source);
            CFRelease(self.run_loop);
        }
    }
}

/// Wakes the service for queued input (called on CoreMIDI's input thread).
pub fn post_input(wake: &RwLock<Option<Arc<Waker>>>) {
    if let Some(w) = wake.read().unwrap_or_else(PoisonError::into_inner).as_ref() {
        w.signal(INPUT);
    }
}

/// The running thread. Dropping it from another thread stops and joins it; dropped on its own
/// thread (the handler said to end) it only detaches.
pub struct Service {
    waker: Arc<Waker>,
    thread: Option<JoinHandle<()>>,
    id: ThreadId,
    queue: Arc<InputQueue>,
}

impl Service {
    /// Starts the thread; `handler` runs on it for every event and returns false to end it.
    pub fn start(
        queue: Arc<InputQueue>,
        handler: impl Fn(Event) -> bool + Send + 'static,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Result<Arc<Waker>, String>>(1);
        let q = queue.clone();
        let thread = std::thread::Builder::new()
            .name("midi".into())
            .spawn(move || run(q, tx, Box::new(handler)))?;
        let id = thread.thread().id();
        match rx.recv() {
            Ok(Ok(waker)) => Ok(Service {
                waker,
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
        self.waker.signal(RECONCILE);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if std::thread::current().id() == self.id {
            // The thread ends by itself after its handler returns.
            return;
        }
        self.waker.signal(QUIT);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        *self
            .queue
            .wake
            .write()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The thread's state, reached from the run loop's callbacks through their `info` pointer.
struct Ctx {
    handler: Box<dyn Fn(Event) -> bool>,
    waker: Option<Arc<Waker>>,
    timer: CFRunLoopTimerRef,
}

impl Ctx {
    /// Runs one event; a false answer stops the run loop (the thread then ends).
    fn handle(&self, ev: Event) -> bool {
        let alive = (self.handler)(ev);
        if !alive {
            unsafe { CFRunLoopStop(CFRunLoopGetCurrent()) };
        }
        alive
    }
}

/// The source's perform callback: the reasons signalled since the last call, in a fixed order.
extern "C" fn perform(info: *mut c_void) {
    // SAFETY: `info` is the thread's `Ctx`, alive while the run loop runs.
    let ctx = unsafe { &*(info as *const Ctx) };
    let Some(waker) = &ctx.waker else { return };
    let flags = waker.flags.swap(0, Ordering::AcqRel);
    if flags & QUIT != 0 {
        unsafe { CFRunLoopStop(CFRunLoopGetCurrent()) };
        return;
    }
    if flags & DEVICES != 0 {
        unsafe {
            CFRunLoopTimerSetNextFireDate(ctx.timer, CFAbsoluteTimeGetCurrent() + DEBOUNCE_S)
        };
    }
    if flags & INPUT != 0 && !ctx.handle(Event::Input) {
        return;
    }
    if flags & RECONCILE != 0 {
        ctx.handle(Event::Reconcile);
    }
}

/// The debounce timer: devices changed 300 ms ago.
extern "C" fn timer_fired(_timer: CFRunLoopTimerRef, info: *mut c_void) {
    // SAFETY: as in `perform`.
    let ctx = unsafe { &*(info as *const Ctx) };
    ctx.handle(Event::Reconcile);
}

/// A far-future fire date: the timer waits until a notification schedules it.
const NEVER: f64 = 1.0e12;

fn run(
    queue: Arc<InputQueue>,
    ready: mpsc::SyncSender<Result<Arc<Waker>, String>>,
    handler: Box<dyn Fn(Event) -> bool>,
) {
    // SAFETY: plain CoreFoundation setup on this thread; the source and the timer are used on
    // this thread only (signalled from others, which CF allows) and invalidated before it ends.
    // `ctx` outlives the run loop's use of it: both are invalidated before `ctx` is dropped.
    unsafe {
        let mut ctx = Box::new(Ctx {
            handler,
            waker: None,
            timer: std::ptr::null_mut(),
        });
        let info = &mut *ctx as *mut Ctx as *mut c_void;
        let run_loop = CFRunLoopGetCurrent();
        let mut sctx = CFRunLoopSourceContext {
            version: 0,
            info,
            retain: None,
            release: None,
            copy_description: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: Some(perform),
        };
        let source = CFRunLoopSourceCreate(std::ptr::null(), 0, &mut sctx);
        let mut tctx = CFRunLoopTimerContext {
            version: 0,
            info,
            retain: None,
            release: None,
            copy_description: None,
        };
        let timer =
            CFRunLoopTimerCreate(std::ptr::null(), NEVER, NEVER, 0, 0, timer_fired, &mut tctx);
        if source.is_null() || timer.is_null() {
            let _ = ready.send(Err("run loop source or timer could not be made".into()));
            return;
        }
        CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode);
        CFRunLoopAddTimer(run_loop, timer, kCFRunLoopDefaultMode);
        ctx.timer = timer;
        // The waker keeps its own references.
        CFRetain(run_loop);
        CFRetain(source);
        let waker = Arc::new(Waker {
            flags: AtomicU8::new(0),
            run_loop,
            source,
        });
        ctx.waker = Some(waker.clone());

        // Hot-plug: CoreMIDI calls back on this run loop.
        let notify_waker = waker.clone();
        let client = coremidi::Client::new_with_notifications(
            "Kinetra",
            move |n: &coremidi::Notification| {
                if matches!(
                    n,
                    coremidi::Notification::SetupChanged
                        | coremidi::Notification::ObjectAdded(_)
                        | coremidi::Notification::ObjectRemoved(_)
                ) {
                    notify_waker.signal(DEVICES);
                }
            },
        );
        if let Err(status) = &client {
            tracing::warn!("MIDI hot-plug notifications unavailable (OSStatus {status})");
        }

        *queue.wake.write().unwrap_or_else(PoisonError::into_inner) = Some(waker.clone());
        let _ = ready.send(Ok(waker));
        drop(ready);

        // The first reconcile opens the ports; input queued before the waker existed runs too.
        if ctx.handle(Event::Reconcile) && ctx.handle(Event::Input) {
            CFRunLoopRun();
        }

        *queue.wake.write().unwrap_or_else(PoisonError::into_inner) = None;
        CFRunLoopTimerInvalidate(timer);
        CFRunLoopSourceInvalidate(source);
        CFRelease(timer);
        CFRelease(source);
        // Never disposed (see the module docs); its callback only signals the waker.
        std::mem::forget(client);
        drop(ctx);
    }
}
