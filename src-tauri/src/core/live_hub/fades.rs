//! Exact visibility for remote control (`docs/CONTROL_PROTOCOL.md` §6.1 `on_air`): a layer taken
//! off air stays **visible** until every Program renderer counted at that moment (not an app
//! preview, not the Preview bus) has reported that its out-animation finished (`gone`), or
//! [`FADE_CAP`] after the clear (the renderer's 3 s out cap plus delivery time), whichever comes
//! first. PANIC (`cut`), a removed layer, a re-take and "no counted renderer" end it at once.
//!
//! Off by default and free while off: only remote control turns tracking on
//! ([`LiveHub::set_fade_tracking`]). While on, every operation that calls the hook compares
//! each user layer's on-air state with the last scan (O(layers), no allocation per unchanged
//! screen beyond the map) and starts fades for layers that just went off air. A fade ends from
//! a renderer's report, a renderer leaving, or ONE sleeping task per hub that wakes at the
//! earliest deadline (none while nothing fades). [`LiveHub::visible`] also checks the deadline,
//! so the answer never depends on the task.
//!
//! When a fade ends, the visibility hook runs (no hub lock held): `AppState` turns it into
//! `StateChange::Visible`.
//!
//! Lock order: `state` before `fades` before the readiness book.

use std::collections::HashSet;

use super::*;

/// The longest a layer counts as visible after it went off air without every renderer
/// reporting: the renderer's out-animation cap (3 s, `OUT_CAP_MS`) plus delivery time.
pub const FADE_CAP: Duration = Duration::from_millis(3500);

#[derive(Default)]
pub(super) struct FadeBook {
    /// Tracking is on (remote control connected).
    enabled: bool,
    /// On air at the last scan, per user layer.
    last: HashMap<LayerKey, bool>,
    fading: HashMap<LayerKey, Fade>,
    /// A task sleeps until the earliest fade's deadline.
    task: bool,
}

struct Fade {
    until: Instant,
    /// Counted renderer connections that haven't reported yet.
    waiting: HashSet<u64>,
}

/// What [`LiveHub::visible_layers`] reports per user layer.
pub type VisibleMap = HashMap<LayerKey, bool>;

fn on_air_map(state: &LiveState) -> HashMap<LayerKey, bool> {
    let mut out = HashMap::new();
    for s in state
        .screens
        .iter()
        .filter(|s| !is_reserved_screen(&s.info.id))
    {
        for l in &s.layers {
            out.insert((s.info.id.clone(), l.id.clone()), l.on_air);
        }
    }
    out
}

impl FadeBook {
    /// Whether a layer that is off air in the state still shows. A transition the last scan
    /// hasn't seen yet (on at the last scan) counts as showing.
    fn shows(&self, key: &LayerKey, now: Instant) -> bool {
        match self.fading.get(key) {
            Some(f) => f.until > now,
            None => self.last.get(key).copied().unwrap_or(false),
        }
    }
}

/// Removes renderer `conn` from every fade; the fades it was the last one for end. `true` if any
/// ended.
pub(super) fn forget_renderer(fades: &Mutex<FadeBook>, conn: u64) -> bool {
    let mut f = lock(fades);
    let before = f.fading.len();
    f.fading.retain(|_, fd| {
        fd.waiting.remove(&conn);
        !fd.waiting.is_empty()
    });
    f.fading.len() != before
}

impl LiveHub {
    /// Turns exact visibility tracking on (remote control connected) or off. Turning it on
    /// starts from the current on-air states with nothing fading.
    pub fn set_fade_tracking(&self, on: bool) {
        let state = self.read();
        let mut f = lock(&self.shared.fades);
        f.enabled = on;
        f.fading.clear();
        f.last = if on {
            on_air_map(&state)
        } else {
            HashMap::new()
        };
    }

    /// Installs the callback run (no hub lock held) when a fade ended without any other live
    /// operation: a renderer reported, left, or the fallback deadline passed.
    pub fn set_visible_hook(&self, hook: LiveHook) {
        *self
            .shared
            .visible_hook
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(hook);
    }

    /// Compares on-air states with the last scan and starts fades (see the module docs). `cut`:
    /// every layer that went off air did so without an out-animation (PANIC), so nothing fades.
    /// Caller holds the state lock (read or write).
    pub(super) fn scan_fades(&self, state: &LiveState, cut: bool) {
        let mut f = lock(&self.shared.fades);
        if !f.enabled {
            return;
        }
        let now = Instant::now();
        let next = on_air_map(state);
        let mut started = false;
        for (key, &on) in &next {
            let was = f.last.get(key).copied().unwrap_or(false);
            if on {
                f.fading.remove(key);
            } else if was && !cut {
                let waiting: HashSet<u64> = lock(&self.shared.book)
                    .conns
                    .iter()
                    .filter(|(_, c)| c.counted && c.screen == key.0)
                    .map(|(id, _)| *id)
                    .collect();
                if !waiting.is_empty() {
                    f.fading.insert(
                        key.clone(),
                        Fade {
                            until: now + FADE_CAP,
                            waiting,
                        },
                    );
                    started = true;
                }
            }
        }
        if cut {
            f.fading.clear();
        }
        f.fading.retain(|k, _| next.contains_key(k));
        f.last = next;
        if started && !f.task {
            if let Some(rt) = self.runtime_handle() {
                f.task = true;
                let fades = self.shared.fades.clone();
                let hook = self.shared.visible_hook.clone();
                rt.spawn(fade_timer(fades, hook));
            }
        }
    }

    /// Brings the fades up to date with the live state (a path that changed on-air states
    /// without the hook would otherwise wait for the next operation).
    pub fn refresh_fades(&self) {
        let state = self.read();
        self.scan_fades(&state, false);
    }

    /// A counted Program renderer finished a layer's out-animation (`{t:"gone"}`).
    pub fn renderer_gone(&self, reg: &RendererReg, layer_id: &str) {
        let Some(screen) = lock(&self.shared.book)
            .conns
            .get(&reg.id)
            .filter(|c| c.counted)
            .map(|c| c.screen.clone())
        else {
            return;
        };
        let ended = {
            let mut f = lock(&self.shared.fades);
            let key = (screen, layer_id.to_string());
            match f.fading.get_mut(&key) {
                Some(fd) => {
                    fd.waiting.remove(&reg.id);
                    if fd.waiting.is_empty() {
                        f.fading.remove(&key);
                        true
                    } else {
                        false
                    }
                }
                None => false,
            }
        };
        if ended {
            call_hook(&self.shared.visible_hook);
        }
    }

    /// Whether a layer can be seen on the output: on air, or going off air with its
    /// out-animation not finished everywhere (with tracking off: on air).
    pub fn visible(&self, screen_id: &str, layer_id: &str) -> bool {
        let state = self.read();
        self.visible_locked(&state, screen_id, layer_id)
    }

    /// [`LiveHub::visible`] for every user layer, under one lock.
    pub fn visible_layers(&self) -> VisibleMap {
        let state = self.read();
        let f = lock(&self.shared.fades);
        let now = Instant::now();
        let mut out = on_air_map(&state);
        if f.enabled {
            for (k, v) in out.iter_mut() {
                *v = *v || f.shows(k, now);
            }
        }
        out
    }

    /// Caller holds the state lock.
    pub(super) fn visible_locked(
        &self,
        state: &LiveState,
        screen_id: &str,
        layer_id: &str,
    ) -> bool {
        let on = state
            .screen(screen_id)
            .and_then(|s| s.layers.iter().find(|l| l.id == layer_id))
            .map(|l| l.on_air);
        match on {
            None => false,
            Some(true) => true,
            Some(false) => {
                let f = lock(&self.shared.fades);
                f.enabled
                    && f.shows(
                        &(screen_id.to_string(), layer_id.to_string()),
                        Instant::now(),
                    )
            }
        }
    }

    /// Whether a layer is cued in on its screen's Preview (Studio mode). Caller holds the state
    /// lock.
    pub(super) fn cued_locked(state: &LiveState, screen_id: &str, layer_id: &str) -> bool {
        state.studio
            && state
                .screen(screen_id)
                .and_then(|s| s.preview.layers.get(layer_id))
                .is_some_and(|c| c.visible == Some(true))
    }

    /// Fades in progress (tests).
    pub fn fading_count(&self) -> usize {
        lock(&self.shared.fades).fading.len()
    }
}

/// Sleeps until the earliest fade deadline, ends the expired fades (running the visibility
/// hook), and stops when nothing fades any more.
async fn fade_timer(fades: Arc<Mutex<FadeBook>>, hook: Arc<HookSlot>) {
    loop {
        let next = {
            let mut f = lock(&fades);
            match f.fading.values().map(|fd| fd.until).min() {
                Some(t) => t,
                None => {
                    f.task = false;
                    return;
                }
            }
        };
        tokio::time::sleep_until(next.into()).await;
        let ended = {
            let mut f = lock(&fades);
            let now = Instant::now();
            let before = f.fading.len();
            f.fading.retain(|_, fd| fd.until > now);
            f.fading.len() != before
        };
        if ended {
            call_hook(&hook);
        }
    }
}
