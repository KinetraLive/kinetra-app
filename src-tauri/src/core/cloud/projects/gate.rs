//! "Is anything on air?" for cloud transfers (CLOUD_PROJECTS.md §0 rule 1, §2.2).
//!
//! On air = a Program layer of a user screen is on air, a display output shows, or a counted
//! Program renderer is connected (OBS / vMix with the browser source loaded; owner decision
//! 2026-10-15: their stream's upload is the real risk). While on air, `pause` (the default)
//! cancels the transfer in flight at once and starts none; hashing of large files waits too.
//! (`limit`, a speed cap instead, comes later; until then it pauses as well.)
//!
//! Cost: the gate is only kept current while a backup or restore runs (`active`); at rest the
//! change listener is one atomic load. Async transfers wait on a `watch`, blocking hashing on a
//! condvar.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use tokio::sync::watch;

use crate::core::state::{AppState, StateChange};

/// The on-air state for transfers.
pub struct Gate {
    /// A backup or restore runs: keep `on_air` current.
    active: AtomicBool,
    on_air: watch::Sender<bool>,
    blocking: (Mutex<bool>, Condvar),
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            active: AtomicBool::new(false),
            on_air: watch::channel(false).0,
            blocking: (Mutex::new(false), Condvar::new()),
        }
    }
}

impl Gate {
    fn set(&self, on: bool) {
        self.on_air.send_if_modified(|v| {
            let changed = *v != on;
            *v = on;
            changed
        });
        let (m, cv) = &self.blocking;
        let mut g = m.lock().unwrap_or_else(PoisonError::into_inner);
        if *g != on {
            *g = on;
            cv.notify_all();
        }
    }

    pub fn is_on_air(&self) -> bool {
        *self.on_air.borrow()
    }

    /// For async transfers: changes of the on-air state.
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.on_air.subscribe()
    }

    /// Blocks while on air (hashing large files), or until `cancel` is set. `false` = canceled.
    pub fn wait_blocking(&self, cancel: &AtomicBool) -> bool {
        let (m, cv) = &self.blocking;
        let mut g = m.lock().unwrap_or_else(PoisonError::into_inner);
        while *g {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            g = cv
                .wait_timeout(g, std::time::Duration::from_millis(500))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        !cancel.load(Ordering::Relaxed)
    }
}

/// Waits until `rx` says off air.
pub async fn off_air(rx: &mut watch::Receiver<bool>) {
    while *rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            return;
        }
    }
}

/// Resolves when `rx` says on air (to cancel a transfer in flight).
pub async fn goes_on_air(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

impl AppState {
    fn cloud_gate(&self) -> &Gate {
        &self.cloud_projects().gate
    }

    /// Whether anything is on air for cloud transfers (see the module docs).
    pub fn cloud_on_air(&self) -> bool {
        self.live().any_on_air() || self.any_output_showing() || self.live().program_renderers() > 0
    }

    /// Re-reads the on-air state into the gate.
    fn cloud_gate_refresh(&self) {
        self.cloud_gate().set(self.cloud_on_air());
    }

    /// Hooks the gate to live changes and renderer connections (once, at startup).
    pub(crate) fn cloud_gate_init(&self) {
        self.add_change_listener(Arc::new(|st: &AppState, ch: StateChange<'_>| {
            if matches!(ch, StateChange::Live) && st.cloud_gate().active.load(Ordering::Relaxed) {
                st.cloud_gate_refresh();
            }
        }));
        let weak = self.downgrade();
        self.live().set_renderers_hook(Arc::new(move || {
            if let Some(st) = weak.upgrade() {
                if st.cloud_gate().active.load(Ordering::Relaxed) {
                    st.cloud_gate_refresh();
                }
            }
        }));
    }

    /// A transfer starts or ends: the gate follows on-air changes only while one runs.
    pub(crate) fn cloud_gate_active(&self, on: bool) {
        self.cloud_gate().active.store(on, Ordering::Relaxed);
        if on {
            self.cloud_gate_refresh();
        }
    }
}
