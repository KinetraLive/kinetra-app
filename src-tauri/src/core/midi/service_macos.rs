//! The MIDI service thread (macOS): a [`LoopThread`](crate::core::runloop::LoopThread) that is
//! woken by CoreMIDI's input thread (input queued), by reconcile requests from other threads, and
//! by hot-plug: a setup change (a device or port added or removed) reports a change, and 300 ms
//! after the last one the ports are re-enumerated (debounce: a plug-in sends several
//! notifications). It blocks in `CFRunLoopRun` between events: no polling, 0 % CPU at rest. It
//! exists only while MIDI is in use (see `core::midi`), like the Windows service (`service.rs`).
//!
//! # Hot-plug notifications
//! CoreMIDI delivers every client's notifications on the run loop that was current when the
//! process made its FIRST client. The service thread comes and goes with MIDI use, so the
//! notification client lives on its own `midi-notify` loop thread instead: started the first time
//! anything touches CoreMIDI ([`ensure_notifications`], before any other client is made), it
//! stays for the life of the process, blocked in its run loop, and forwards each change to the
//! service running then. One CoreMIDI client, never disposed (disposing an app's last client can
//! stop the MIDI server, after which no client can be made).

use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use super::InputQueue;
use crate::core::runloop::{self, LoopThread};

pub use runloop::Waker;

/// The hot-plug debounce.
const DEBOUNCE_S: f64 = 0.3;

const INPUT: u8 = 1;
const RECONCILE: u8 = 2;

/// What the service thread asks its handler to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Run the queued input.
    Input,
    /// Re-evaluate the ports (a request, or devices changed).
    Reconcile,
}

/// The service that gets hot-plug changes now (none while MIDI is unused).
static TARGET: RwLock<Option<Arc<Waker>>> = RwLock::new(None);
/// The `midi-notify` thread with the notification client (see the module docs).
static NOTIFY: OnceLock<Option<LoopThread>> = OnceLock::new();

/// Makes sure the notification client exists (the process's first CoreMIDI client). Call before
/// anything else uses CoreMIDI; cheap after the first call.
pub fn ensure_notifications() {
    NOTIFY.get_or_init(|| {
        let started = LoopThread::start(
            "midi-notify",
            DEBOUNCE_S,
            |_| {
                let client = coremidi::Client::new_with_notifications(
                    "Kinetra",
                    |n: &coremidi::Notification| {
                        if matches!(
                            n,
                            coremidi::Notification::SetupChanged
                                | coremidi::Notification::ObjectAdded(_)
                                | coremidi::Notification::ObjectRemoved(_)
                        ) {
                            if let Some(w) = TARGET
                                .read()
                                .unwrap_or_else(PoisonError::into_inner)
                                .as_ref()
                            {
                                w.changed();
                            }
                        }
                    },
                );
                if let Err(status) = &client {
                    tracing::warn!("MIDI hot-plug notifications unavailable (OSStatus {status})");
                }
                Box::new(client)
            },
            |_| true,
        );
        match started {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!("MIDI hot-plug thread: {e}");
                None
            }
        }
    });
}

/// Wakes the service for queued input (called on CoreMIDI's input thread).
pub fn post_input(wake: &RwLock<Option<Arc<Waker>>>) {
    if let Some(w) = wake.read().unwrap_or_else(PoisonError::into_inner).as_ref() {
        w.wake(INPUT);
    }
}

/// The running thread. Dropping it from another thread stops and joins it; dropped on its own
/// thread (the handler said to end) it only detaches.
pub struct Service {
    thread: LoopThread,
    queue: Arc<InputQueue>,
}

impl Service {
    /// Starts the thread; `handler` runs on it for every event and returns false to end it.
    pub fn start(
        queue: Arc<InputQueue>,
        handler: impl Fn(Event) -> bool + Send + 'static,
    ) -> std::io::Result<Self> {
        ensure_notifications();
        let q = queue.clone();
        let thread = LoopThread::start(
            "midi",
            DEBOUNCE_S,
            move |waker| {
                *TARGET.write().unwrap_or_else(PoisonError::into_inner) = Some(waker.clone());
                *q.wake.write().unwrap_or_else(PoisonError::into_inner) = Some(waker.clone());
                // The first reconcile opens the ports; input queued before the waker existed
                // runs too.
                waker.wake(RECONCILE | INPUT);
                Box::new(())
            },
            move |ev| match ev {
                runloop::Event::Woken(bits) => {
                    // Ports first: input may be for a port this reconcile opens.
                    (bits & RECONCILE == 0 || handler(Event::Reconcile))
                        && (bits & INPUT == 0 || handler(Event::Input))
                }
                runloop::Event::Settled => handler(Event::Reconcile),
            },
        )?;
        Ok(Self { thread, queue })
    }

    /// Asks the thread to re-evaluate the ports.
    pub fn reconcile(&self) {
        self.thread.waker().wake(RECONCILE);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        // Stopping happens when `thread` drops (after this); hot-plug and input forget it now.
        {
            let mut t = TARGET.write().unwrap_or_else(PoisonError::into_inner);
            if t.as_ref()
                .is_some_and(|w| Arc::ptr_eq(w, self.thread.waker()))
            {
                *t = None;
            }
        }
        *self
            .queue
            .wake
            .write()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }
}
