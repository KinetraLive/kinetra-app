//! The MIDI service thread (macOS): a [`LoopThread`](crate::core::runloop::LoopThread) that
//! - holds a CoreMIDI client with notifications; a setup change (a device or port added or
//!   removed) reports a change, and 300 ms after the last one the ports are re-enumerated
//!   (debounce: a plug-in sends several notifications);
//! - is woken by CoreMIDI's input thread (input queued) and by reconcile requests from other
//!   threads.
//!
//! It blocks in `CFRunLoopRun` between events: no polling, 0 % CPU at rest. It exists only while
//! MIDI is in use (see `core::midi`). The same shape as the Windows service (`service.rs`).
//!
//! CoreMIDI clients are never disposed (disposing an app's last client can stop the MIDI server,
//! after which no client can be made): one notification client per service start, which happens
//! when MIDI comes into use.

use std::sync::{Arc, PoisonError, RwLock};

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
        let q = queue.clone();
        let thread = LoopThread::start(
            "midi",
            DEBOUNCE_S,
            move |waker| {
                // Hot-plug: CoreMIDI calls back on this run loop; the callback only reports.
                let w = waker.clone();
                let client = coremidi::Client::new_with_notifications(
                    "Kinetra",
                    move |n: &coremidi::Notification| {
                        if matches!(
                            n,
                            coremidi::Notification::SetupChanged
                                | coremidi::Notification::ObjectAdded(_)
                                | coremidi::Notification::ObjectRemoved(_)
                        ) {
                            w.changed();
                        }
                    },
                );
                if let Err(status) = &client {
                    tracing::warn!("MIDI hot-plug notifications unavailable (OSStatus {status})");
                }
                *q.wake.write().unwrap_or_else(PoisonError::into_inner) = Some(waker.clone());
                // The first reconcile opens the ports; input queued before the waker existed
                // runs too.
                waker.wake(RECONCILE | INPUT);
                // Kept until the loop ends (never disposed: see the module docs).
                Box::new(client)
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
        // Stopping happens when `thread` drops (after this); the queue forgets the waker now.
        *self
            .queue
            .wake
            .write()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }
}
