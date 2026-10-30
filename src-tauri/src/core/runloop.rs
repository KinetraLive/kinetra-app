//! A service thread on a CoreFoundation run loop (macOS): what the MIDI service and the display
//! watcher run on, in place of the Windows message-only / hidden windows.
//!
//! The thread blocks in `CFRunLoopRun` between events: no polling, 0 % CPU at rest. Other
//! threads wake it through a [`Waker`] (a run-loop source, signalled from any thread) with
//! reasons ([`Waker::wake`]), or report "something changed" ([`Waker::changed`]), which (re)starts
//! a debounce timer; the handler then gets one [`Event::Settled`] for a burst of changes. OS
//! notification callbacks (CoreMIDI, CoreGraphics) only call the waker, so it doesn't matter
//! which thread or run loop the OS delivers them on.

use std::any::Any;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{JoinHandle, ThreadId};

/// What the handler is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Woken with these reasons (the caller's bits, OR-ed since the last wake).
    Woken(u8),
    /// The debounce time passed since the last [`Waker::changed`].
    Settled,
}

/// The reasons callers may use (the top two bits are the loop's own).
pub const USER_BITS: u8 = 0b0011_1111;
const CHANGED: u8 = 0b0100_0000;
const QUIT: u8 = 0b1000_0000;

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

/// Wakes a [`LoopThread`]. Holds its own references to the run loop and the source, so waking
/// after the thread ended is harmless.
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
    fn signal(&self, bits: u8) {
        self.flags.fetch_or(bits, Ordering::AcqRel);
        unsafe {
            CFRunLoopSourceSignal(self.source);
            CFRunLoopWakeUp(self.run_loop);
        }
    }

    /// Wakes the thread with `reasons` (within [`USER_BITS`]).
    pub fn wake(&self, reasons: u8) {
        self.signal(reasons & USER_BITS);
    }

    /// Something changed: (re)starts the debounce timer.
    pub fn changed(&self) {
        self.signal(CHANGED);
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

/// The running thread. Dropping it from another thread stops and joins it; dropped on its own
/// thread (the handler said to end) it only detaches.
pub struct LoopThread {
    waker: Arc<Waker>,
    thread: Option<JoinHandle<()>>,
    id: ThreadId,
}

impl LoopThread {
    /// Starts a thread named `name`. On it, `setup` runs first with the waker (register OS
    /// notifications there; what it returns is kept alive until the loop ends and dropped on the
    /// thread), then the loop; `handler` runs for every event and returns false to end it.
    pub fn start(
        name: &str,
        debounce_s: f64,
        setup: impl FnOnce(&Arc<Waker>) -> Box<dyn Any> + Send + 'static,
        handler: impl Fn(Event) -> bool + Send + 'static,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Result<Arc<Waker>, String>>(1);
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || run(tx, debounce_s, Box::new(setup), Box::new(handler)))?;
        let id = thread.thread().id();
        match rx.recv() {
            Ok(Ok(waker)) => Ok(Self {
                waker,
                thread: Some(thread),
                id,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(std::io::Error::other(e))
            }
            Err(_) => Err(std::io::Error::other(format!("{name} thread ended"))),
        }
    }

    pub fn waker(&self) -> &Arc<Waker> {
        &self.waker
    }
}

impl Drop for LoopThread {
    fn drop(&mut self) {
        if std::thread::current().id() == self.id {
            return;
        }
        self.waker.signal(QUIT);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The thread's state, reached from the run loop's callbacks through their `info` pointer.
struct Ctx {
    handler: Box<dyn Fn(Event) -> bool>,
    waker: Option<Arc<Waker>>,
    timer: CFRunLoopTimerRef,
    debounce_s: f64,
}

impl Ctx {
    fn handle(&self, ev: Event) {
        if !(self.handler)(ev) {
            unsafe { CFRunLoopStop(CFRunLoopGetCurrent()) };
        }
    }
}

extern "C" fn perform(info: *mut c_void) {
    // SAFETY: `info` is the thread's `Ctx`, alive while the run loop runs.
    let ctx = unsafe { &*(info as *const Ctx) };
    let Some(waker) = &ctx.waker else { return };
    let flags = waker.flags.swap(0, Ordering::AcqRel);
    if flags & QUIT != 0 {
        unsafe { CFRunLoopStop(CFRunLoopGetCurrent()) };
        return;
    }
    if flags & CHANGED != 0 {
        unsafe {
            CFRunLoopTimerSetNextFireDate(ctx.timer, CFAbsoluteTimeGetCurrent() + ctx.debounce_s)
        };
    }
    if flags & USER_BITS != 0 {
        ctx.handle(Event::Woken(flags & USER_BITS));
    }
}

extern "C" fn timer_fired(_timer: CFRunLoopTimerRef, info: *mut c_void) {
    // SAFETY: as in `perform`.
    let ctx = unsafe { &*(info as *const Ctx) };
    ctx.handle(Event::Settled);
}

/// A far-future fire date: the timer waits until a change schedules it.
const NEVER: f64 = 1.0e12;

type Setup = Box<dyn FnOnce(&Arc<Waker>) -> Box<dyn Any> + Send>;

fn run(
    ready: mpsc::SyncSender<Result<Arc<Waker>, String>>,
    debounce_s: f64,
    setup: Setup,
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
            debounce_s,
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
        let keep = setup(&waker);
        let _ = ready.send(Ok(waker));
        drop(ready);

        CFRunLoopRun();

        drop(keep);
        CFRunLoopTimerInvalidate(timer);
        CFRunLoopSourceInvalidate(source);
        CFRelease(timer);
        CFRelease(source);
        drop(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn wakes_with_reasons_debounces_changes_and_stops() {
        let (tx, rx) = mpsc::channel();
        let t = LoopThread::start(
            "test-loop",
            0.2,
            |_| Box::new(()),
            move |ev| tx.send(ev).is_ok(),
        )
        .unwrap();
        let next = |s| rx.recv_timeout(Duration::from_secs(s));
        t.waker().wake(1);
        assert_eq!(next(2).unwrap(), Event::Woken(1));
        // A burst of changes: one Settled, after the debounce.
        let t0 = Instant::now();
        for _ in 0..5 {
            t.waker().changed();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(next(2).unwrap(), Event::Settled);
        assert!(
            t0.elapsed() >= Duration::from_millis(250),
            "{:?}",
            t0.elapsed()
        );
        assert!(next(1).is_err(), "one Settled for the burst");
        drop(t);
    }

    #[test]
    fn a_false_answer_ends_the_thread() {
        let (tx, rx) = mpsc::channel();
        let t = LoopThread::start(
            "test-loop",
            0.2,
            |_| Box::new(()),
            move |ev| {
                let _ = tx.send(ev);
                false
            },
        )
        .unwrap();
        t.waker().wake(2);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Event::Woken(2)
        );
        // Ended: further wakes reach nobody, and dropping the handle doesn't hang.
        t.waker().wake(2);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        drop(t);
    }
}
