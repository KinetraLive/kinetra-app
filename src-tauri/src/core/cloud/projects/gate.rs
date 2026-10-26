//! "Is anything on air?" for cloud transfers (CLOUD_PROJECTS.md §0 rule 1, §2.2).
//!
//! On air = a Program layer of a user screen is on air, a display output shows, or a counted
//! Program renderer is connected (OBS / vMix with the browser source loaded; owner decision
//! 2026-10-15: their stream's upload is the real risk). What happens then is the on-air policy
//! (this computer's setting, else the server's `client.on_air`):
//! - `pause` (the default): a transfer in flight is dropped at once and none starts; hashing of
//!   large files waits too;
//! - `limit`: transfers go on, paced to `limit_kbps` (`Pacer`), and hashing goes on.
//!
//! The gate turns both into one [`Mode`] that transfers watch, so a change of either (going on
//! air, or the setting changing mid-backup) takes effect at once.
//!
//! Cost: the on-air state is only kept current while a backup or restore runs (`active`); at
//! rest the change listener is one atomic load. Async transfers wait on a `watch`, blocking
//! hashing on a condvar.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::watch;

use super::wire::OnAir;
use crate::core::state::{AppState, StateChange};

/// What transfers may do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nothing on air: full speed.
    Free,
    /// On air, `pause`: nothing is sent.
    Pause,
    /// On air, `limit`: at most this many kbit/s.
    Limit(u32),
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The on-air state and policy for transfers.
pub struct Gate {
    /// A backup or restore runs: keep `on_air` current.
    active: AtomicBool,
    on_air: AtomicBool,
    policy: Mutex<(OnAir, u32)>,
    mode: watch::Sender<Mode>,
    blocking: (Mutex<Mode>, Condvar),
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            active: AtomicBool::new(false),
            on_air: AtomicBool::new(false),
            policy: Mutex::new((OnAir::Pause, 2000)),
            mode: watch::channel(Mode::Free).0,
            blocking: (Mutex::new(Mode::Free), Condvar::new()),
        }
    }
}

impl Gate {
    fn recompute(&self) {
        let mode = if !self.on_air.load(Ordering::Relaxed) {
            Mode::Free
        } else {
            match *lock(&self.policy) {
                (OnAir::Pause, _) => Mode::Pause,
                (OnAir::Limit, kbps) => Mode::Limit(kbps.max(1)),
            }
        };
        self.mode.send_if_modified(|m| {
            let changed = *m != mode;
            *m = mode;
            changed
        });
        let (m, cv) = &self.blocking;
        let mut g = lock(m);
        if *g != mode {
            *g = mode;
            cv.notify_all();
        }
    }

    fn set_on_air(&self, on: bool) {
        self.on_air.store(on, Ordering::Relaxed);
        self.recompute();
    }

    /// The on-air policy (this computer's setting, else the server's default).
    pub fn set_policy(&self, on_air: OnAir, kbps: u32) {
        *lock(&self.policy) = (on_air, kbps);
        self.recompute();
    }

    pub fn mode(&self) -> Mode {
        *self.mode.borrow()
    }

    /// For async transfers: changes of the mode.
    pub fn subscribe(&self) -> watch::Receiver<Mode> {
        self.mode.subscribe()
    }

    /// Blocks while transfers must pause (hashing large files), or until `cancel` is set.
    /// `false` = canceled.
    pub fn wait_blocking(&self, cancel: &AtomicBool) -> bool {
        let (m, cv) = &self.blocking;
        let mut g = lock(m);
        while *g == Mode::Pause {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            g = cv
                .wait_timeout(g, Duration::from_millis(500))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        !cancel.load(Ordering::Relaxed)
    }
}

/// Waits until sending is allowed (not `Pause`).
pub async fn may_send(rx: &mut watch::Receiver<Mode>) {
    while *rx.borrow_and_update() == Mode::Pause {
        if rx.changed().await.is_err() {
            return;
        }
    }
}

/// Resolves when transfers must pause (to drop one in flight).
pub async fn must_pause(rx: &mut watch::Receiver<Mode>) {
    loop {
        if *rx.borrow_and_update() == Mode::Pause {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Paces a byte stream to the `Limit` rate while that mode lasts (full speed otherwise). Each
/// chunk waits until the bytes sent so far (this one included) fit the rate since pacing began,
/// so even a single chunk is held back.
#[derive(Default)]
pub struct Pacer {
    since: Option<(Instant, u64, u32)>,
}

impl Pacer {
    /// How long to wait before sending `n` more bytes in `mode`.
    pub fn delay(&mut self, mode: Mode, n: u64, now: Instant) -> Duration {
        let Mode::Limit(kbps) = mode else {
            self.since = None;
            return Duration::ZERO;
        };
        let (start, sent, rate) = match self.since {
            Some((s, b, r)) if r == kbps => (s, b, r),
            _ => (now, 0, kbps),
        };
        let sent = sent + n;
        self.since = Some((start, sent, rate));
        let bytes_per_s = f64::from(rate) * 1000.0 / 8.0;
        let due = start + Duration::from_secs_f64(sent as f64 / bytes_per_s);
        due.saturating_duration_since(now)
    }
}

impl AppState {
    pub(super) fn cloud_gate(&self) -> &Gate {
        &self.cloud_projects().gate
    }

    /// Whether anything is on air for cloud transfers (see the module docs).
    pub fn cloud_on_air(&self) -> bool {
        self.live().any_on_air() || self.any_output_showing() || self.live().program_renderers() > 0
    }

    /// Re-reads the on-air state into the gate.
    fn cloud_gate_refresh(&self) {
        self.cloud_gate().set_on_air(self.cloud_on_air());
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

    /// An output started or stopped counting as on air (a display output, or an NDI output's
    /// first receiver / last one gone).
    pub(crate) fn cloud_outputs_changed(&self) {
        if self.cloud_gate().active.load(Ordering::Relaxed) {
            self.cloud_gate_refresh();
        }
    }

    /// A transfer starts or ends: the gate follows on-air changes only while one runs.
    pub(crate) fn cloud_gate_active(&self, on: bool) {
        self.cloud_gate().active.store(on, Ordering::Relaxed);
        if on {
            let s = self.cloud_view().settings;
            self.cloud_gate().set_policy(s.on_air, s.limit_kbps);
            self.cloud_gate_refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_follow_on_air_and_the_policy() {
        let g = Gate::default();
        assert_eq!(g.mode(), Mode::Free);
        g.set_on_air(true);
        assert_eq!(g.mode(), Mode::Pause);
        g.set_policy(OnAir::Limit, 500);
        assert_eq!(g.mode(), Mode::Limit(500));
        g.set_on_air(false);
        assert_eq!(g.mode(), Mode::Free);
        // The setting changes while on air: at once.
        g.set_on_air(true);
        g.set_policy(OnAir::Pause, 500);
        assert_eq!(g.mode(), Mode::Pause);
    }

    #[test]
    fn the_pacer_holds_bytes_to_the_rate_only_while_limited() {
        let mut p = Pacer::default();
        let t0 = Instant::now();
        // Free: no wait.
        assert_eq!(p.delay(Mode::Free, 1_000_000, t0), Duration::ZERO);
        // 800 kbit/s = 100 000 B/s: 50 000 B is due half a second after pacing began.
        let d = p.delay(Mode::Limit(800), 50_000, t0);
        assert_eq!(d, Duration::from_millis(500));
        let d = p.delay(Mode::Limit(800), 50_000, t0 + Duration::from_millis(500));
        assert_eq!(d, Duration::from_millis(500));
        // A new rate starts over.
        let d = p.delay(Mode::Limit(1600), 200_000, t0 + Duration::from_secs(1));
        assert_eq!(d, Duration::from_secs(1));
        // Off air again: full speed, and the next limit starts fresh.
        assert_eq!(p.delay(Mode::Free, 10, t0), Duration::ZERO);
    }
}
