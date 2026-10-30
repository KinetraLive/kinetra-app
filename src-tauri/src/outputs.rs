//! Display output windows (Tauri side of `core::outputs`): borderless, undecorated, not in the
//! taskbar, not focusable, positioned on their display (Windows: physical pixels; macOS: points,
//! see `core::displays::macos`), cursor hidden. They
//! load the output server (a remote URL) or the app's idle page and are in no capability, so
//! they can't call commands. The user can't close them (`CloseRequested` is prevented in
//! lib.rs); Kinetra destroys them.
//!
//! **Audio:** each output window grants the output server's origin the stored
//! microphone and autoplay permission states (`webview2::allow_output_media`), so the renderer
//! plays media unmuted from Take and can list audio outputs and `setSinkId` to the one the
//! output names (`&sink=<label>`). Nothing is ever captured. A port change resets the old origin.
//!
//! **Crash recovery:** an output window whose page dies (render process exited or unresponsive,
//! browser process exited) is destroyed and created again from its plan (the renderer
//! reconnects and gets a snapshot). At most [`CRASH_RECREATIONS_MAX`] times per
//! [`CRASH_WINDOW`] per output window; after that the window stays closed and the output
//! reports `failed` "The output page keeps crashing" until its plan changes (turned off and on,
//! another screen, display, port…). Windows only so far (WebView2 reports the failure).
//!
//! **macOS:** an output window sits at the screen-saver window level, above the menu bar and the
//! Dock of its display, on every Space, out of ⌘` and Mission Control, and is ordered in
//! without activating Kinetra (no native full screen: it would make its own Space and animate).
//!
//! **Displays stay awake** while any output window shows ([`crate::power::DisplayAwake`]).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use tauri::window::Color;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow};
#[cfg(target_os = "macos")]
use tauri::{LogicalPosition, LogicalSize};
#[cfg(not(target_os = "macos"))]
use tauri::{PhysicalPosition, PhysicalSize};

use crate::commands::app::MAIN_WINDOW;
use crate::core::outputs::{
    OutputHost, WindowPlan, WindowUrl, IDENTIFY_LABEL_PREFIX, OUTPUT_LABEL_PREFIX, PAGE_CRASHING,
};
use crate::core::state::AppState;
use crate::power::DisplayAwake;
use crate::webview2::{self, PageFailure};

/// Recreations of one output window allowed within [`CRASH_WINDOW`].
pub const CRASH_RECREATIONS_MAX: usize = 3;
pub const CRASH_WINDOW: Duration = Duration::from_secs(60);
/// How long a destroyed window may take to leave Tauri's window list before it is recreated.
const DESTROY_WAIT: Duration = Duration::from_secs(2);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether a window label is an output or Identify window (not closable by the user).
pub fn is_output_label(label: &str) -> bool {
    label.starts_with(OUTPUT_LABEL_PREFIX) || label.starts_with(IDENTIFY_LABEL_PREFIX)
}

/// Recent recreations of one window; `true` when one more is allowed at `now` (and records it).
fn allow_recreation(history: &mut VecDeque<Instant>, now: Instant) -> bool {
    while history
        .front()
        .is_some_and(|t| now.duration_since(*t) >= CRASH_WINDOW)
    {
        history.pop_front();
    }
    if history.len() >= CRASH_RECREATIONS_MAX {
        return false;
    }
    history.push_back(now);
    true
}

/// The windows Kinetra shows on displays.
pub struct OutputWindows {
    app: AppHandle,
    me: Weak<Self>,
    /// What each output window was last given. Locked briefly, never across a window call.
    shown: Mutex<HashMap<String, WindowPlan>>,
    /// Identify windows shown, and a generation so a newer Identify's timer wins.
    identify: Mutex<Vec<String>>,
    identify_gen: Arc<AtomicU64>,
    /// Serializes window work of reconciles and crash recovery (both on the blocking pool; never
    /// taken on the main thread).
    work: Mutex<()>,
    /// The instance each output window label currently is (a crash of an older one is ignored).
    instances: Mutex<HashMap<String, u64>>,
    next_instance: AtomicU64,
    /// Recreations per output window within the last [`CRASH_WINDOW`].
    crashes: Mutex<HashMap<String, VecDeque<Instant>>>,
    /// Output windows given up on, with the plan they kept crashing with.
    gave_up: Mutex<HashMap<String, WindowPlan>>,
    /// The output server origin granted audio devices.
    audio_origin: Mutex<Option<String>>,
    /// Held while any output window shows.
    awake: Mutex<Option<DisplayAwake>>,
}

impl OutputWindows {
    pub fn new(app: AppHandle) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            app,
            me: me.clone(),
            shown: Mutex::new(HashMap::new()),
            identify: Mutex::new(Vec::new()),
            identify_gen: Arc::new(AtomicU64::new(0)),
            work: Mutex::new(()),
            instances: Mutex::new(HashMap::new()),
            next_instance: AtomicU64::new(0),
            crashes: Mutex::new(HashMap::new()),
            gave_up: Mutex::new(HashMap::new()),
            audio_origin: Mutex::new(None),
            awake: Mutex::new(None),
        })
    }

    /// The URL a window loads. App pages resolve against the main window's origin (the dev
    /// server in debug, `http://tauri.localhost` in release).
    fn url(&self, url: &WindowUrl) -> Result<Url, String> {
        match url {
            WindowUrl::Http(s) => Url::parse(s).map_err(|e| e.to_string()),
            WindowUrl::App(path) => {
                let base = self
                    .app
                    .get_webview_window(MAIN_WINDOW)
                    .and_then(|w| w.url().ok())
                    .ok_or("The main window is gone")?;
                base.join(&format!("/{path}")).map_err(|e| e.to_string())
            }
        }
    }

    fn create(&self, p: &WindowPlan) -> Result<WebviewWindow, String> {
        let url = match &p.url {
            WindowUrl::Http(_) => WebviewUrl::External(self.url(&p.url)?),
            WindowUrl::App(path) => WebviewUrl::App(path.into()),
        };
        let w = tauri::WebviewWindowBuilder::new(&self.app, &p.label, url)
            .title("Kinetra output")
            .decorations(false)
            .shadow(false)
            .skip_taskbar(true)
            .resizable(false)
            .focused(false)
            .focusable(false)
            .always_on_top(p.always_on_top)
            .background_color(Color(0, 0, 0, 255))
            .visible(false)
            .build()
            .map_err(|e| e.to_string())?;
        if p.label.starts_with(OUTPUT_LABEL_PREFIX) {
            self.grant_audio(&w, &p.url);
            self.watch_crashes(&w);
        }
        place(&w, p);
        let _ = w.set_cursor_visible(false);
        #[cfg(target_os = "macos")]
        macos_output_window(&w, true);
        #[cfg(not(target_os = "macos"))]
        w.show().map_err(|e| e.to_string())?;
        // A display with another scale factor may have resized it while it moved there.
        place(&w, p);
        Ok(w)
    }

    fn update(&self, w: &WebviewWindow, old: &WindowPlan, p: &WindowPlan) -> Result<(), String> {
        if old.rect != p.rect {
            place(w, p);
        }
        if old.always_on_top != p.always_on_top {
            w.set_always_on_top(p.always_on_top)
                .map_err(|e| e.to_string())?;
            // macOS keeps output windows above the menu bar either way.
            #[cfg(target_os = "macos")]
            macos_output_window(w, false);
        }
        if old.url != p.url {
            self.grant_audio(w, &p.url);
            w.navigate(self.url(&p.url)?).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn destroy(&self, label: &str) {
        if let Some(w) = self.app.get_webview_window(label) {
            if let Err(e) = w.destroy() {
                tracing::warn!("closing {label}: {e}");
            }
        }
    }

    /// Destroys a window and waits (briefly) until its label is free again.
    fn destroy_and_wait(&self, label: &str) {
        self.destroy(label);
        let until = Instant::now() + DESTROY_WAIT;
        while self.app.get_webview_window(label).is_some() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Holds the displays awake while any output window shows.
    fn keep_awake(&self, showing: bool) {
        let mut awake = lock(&self.awake);
        match (showing, awake.is_some()) {
            (true, false) => *awake = DisplayAwake::hold("Kinetra is showing an output"),
            (false, true) => *awake = None,
            _ => {}
        }
    }

    /// Lets the output server's pages see audio devices (once per origin; a new port resets the
    /// old one). The permission lives in the profile every window shares.
    fn grant_audio(&self, w: &WebviewWindow, url: &WindowUrl) {
        let WindowUrl::Http(s) = url else { return };
        let Some(origin) = Url::parse(s).ok().as_ref().and_then(webview2::origin_of) else {
            return;
        };
        let old = {
            let mut g = lock(&self.audio_origin);
            if g.as_deref() == Some(origin.as_str()) {
                return;
            }
            g.replace(origin.clone())
        };
        if let Some(old) = old {
            webview2::allow_output_media(w, &old, false);
        }
        webview2::allow_output_media(w, &origin, true);
    }

    /// Recovers the window when its page dies (see the module docs).
    fn watch_crashes(&self, w: &WebviewWindow) {
        let label = w.label().to_string();
        let instance = self.next_instance.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&self.instances).insert(label.clone(), instance);
        let me = self.me.clone();
        webview2::on_process_failed(w, move |failure| {
            // On the main thread, inside WebView2's event: recover on the blocking pool.
            let (me, label) = (me.clone(), label.clone());
            tauri::async_runtime::spawn_blocking(move || {
                if let Some(me) = me.upgrade() {
                    me.recover(&label, instance, failure);
                }
            });
        });
    }

    fn recover(&self, label: &str, instance: u64, failure: PageFailure) {
        let work = lock(&self.work);
        if lock(&self.instances).get(label) != Some(&instance) {
            return; // already replaced
        }
        let Some(plan) = lock(&self.shown).get(label).cloned() else {
            return; // no longer shown
        };
        let again = allow_recreation(
            lock(&self.crashes).entry(label.to_string()).or_default(),
            Instant::now(),
        );
        tracing::warn!(
            "output window {label}: page failed ({failure:?}); {}",
            if again { "recreating it" } else { "giving up" }
        );
        lock(&self.instances).remove(label);
        self.destroy_and_wait(label);
        let tell_core = if again {
            match self.create(&plan) {
                Ok(_) => false,
                Err(e) => {
                    // Like a failed reconcile: the next one reports it and retries.
                    tracing::warn!("output window {label}: {e}");
                    lock(&self.shown).remove(label);
                    self.destroy(label);
                    true
                }
            }
        } else {
            lock(&self.shown).remove(label);
            lock(&self.gave_up).insert(label.to_string(), plan);
            true
        };
        drop(work);
        if tell_core {
            if let Some(st) = self.app.try_state::<AppState>() {
                st.outputs_host_changed();
            }
        }
    }
}

/// Moves and sizes a window onto its rect (Windows: physical pixels; macOS: points).
fn place(w: &WebviewWindow, p: &WindowPlan) {
    let r = p.rect;
    #[cfg(not(target_os = "macos"))]
    let (pos, size) = (
        w.set_position(PhysicalPosition::new(r.x, r.y)),
        w.set_size(PhysicalSize::new(r.width, r.height)),
    );
    #[cfg(target_os = "macos")]
    let (pos, size) = (
        w.set_position(LogicalPosition::new(f64::from(r.x), f64::from(r.y))),
        w.set_size(LogicalSize::new(f64::from(r.width), f64::from(r.height))),
    );
    if let Err(e) = pos {
        tracing::warn!("{}: position: {e}", p.label);
    }
    if let Err(e) = size {
        tracing::warn!("{}: size: {e}", p.label);
    }
}

/// macOS: the screen-saver level (above the menu bar and the Dock of its display), on every
/// Space and out of ⌘` / Mission Control, never hidden when Kinetra is in the background;
/// `order_in` also shows it without activating Kinetra (the operator keeps their window).
#[cfg(target_os = "macos")]
fn macos_output_window(w: &WebviewWindow, order_in: bool) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    let Ok(ptr) = w.ns_window() else { return };
    let ptr = ptr as usize;
    let r = w.run_on_main_thread(move || {
        let win = ptr as *mut AnyObject;
        // SAFETY: the NSWindow of a live Tauri window, on the main thread.
        unsafe {
            // NSScreenSaverWindowLevel
            let _: () = msg_send![win, setLevel: 1000isize];
            // canJoinAllSpaces | stationary | ignoresCycle | fullScreenAuxiliary
            let _: () = msg_send![win, setCollectionBehavior: 1usize | 16 | 64 | 256];
            let _: () = msg_send![win, setHidesOnDeactivate: false];
            let _: () = msg_send![win, setCanHide: false];
            if order_in {
                let _: () = msg_send![win, orderFrontRegardless];
            }
        }
    });
    if let Err(e) = r {
        tracing::warn!("{}: window setup: {e}", w.label());
    }
}

impl OutputHost for OutputWindows {
    fn apply(&self, windows: &[WindowPlan]) -> HashMap<String, String> {
        let _work = lock(&self.work);
        // A window given up on stays closed while its plan is unchanged; any change (or the
        // output turned off) gives it a fresh start.
        {
            let mut gave_up = lock(&self.gave_up);
            let mut crashes = lock(&self.crashes);
            gave_up.retain(|_, plan| windows.contains(plan));
            crashes.retain(|label, _| {
                gave_up.contains_key(label) || windows.iter().any(|w| w.label == *label)
            });
        }
        let gave_up = lock(&self.gave_up).clone();
        let shown = lock(&self.shown).clone();
        let mut failures = HashMap::new();
        for label in shown.keys() {
            if !windows.iter().any(|w| w.label == *label) {
                self.destroy(label);
            }
        }
        let mut now = HashMap::new();
        for p in windows {
            if gave_up.contains_key(&p.label) {
                failures.insert(p.label.clone(), PAGE_CRASHING.to_string());
                continue;
            }
            let existing = self.app.get_webview_window(&p.label);
            let result = match (existing, shown.get(&p.label)) {
                (Some(w), Some(old)) => self.update(&w, old, p),
                (Some(w), None) => {
                    // Left over (not ours to update): start over.
                    let _ = w.destroy();
                    self.create(p).map(|_| ())
                }
                (None, _) => self.create(p).map(|_| ()),
            };
            match result {
                Ok(()) => {
                    now.insert(p.label.clone(), p.clone());
                }
                Err(e) => {
                    self.destroy(&p.label);
                    failures.insert(p.label.clone(), e);
                }
            }
        }
        let showing = !now.is_empty();
        *lock(&self.shown) = now;
        self.keep_awake(showing);
        failures
    }

    fn identify(&self, windows: Vec<WindowPlan>, duration: Duration) {
        let old = std::mem::take(&mut *lock(&self.identify));
        for label in &old {
            self.destroy(label);
        }
        let gen = self.identify_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let mut labels = Vec::new();
        for p in &windows {
            match self.create(p) {
                Ok(_) => labels.push(p.label.clone()),
                Err(e) => tracing::warn!("identify window {}: {e}", p.label),
            }
        }
        *lock(&self.identify) = labels;
        let app = self.app.clone();
        let current = self.identify_gen.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(duration).await;
            if current.load(Ordering::SeqCst) != gen {
                return; // a newer Identify replaced these
            }
            for (label, w) in app.webview_windows() {
                if label.starts_with(IDENTIFY_LABEL_PREFIX) {
                    let _ = w.destroy();
                }
            }
        });
    }

    fn close_all(&self) {
        self.identify_gen.fetch_add(1, Ordering::SeqCst);
        lock(&self.shown).clear();
        self.keep_awake(false);
        lock(&self.identify).clear();
        // Closing is not a crash: no recovery for these instances.
        lock(&self.instances).clear();
        for (label, w) in self.app.webview_windows() {
            if is_output_label(&label) {
                let _ = w.destroy();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_labels() {
        assert!(is_output_label("output-abc"));
        assert!(is_output_label("identify-1"));
        assert!(!is_output_label("main"));
        assert!(!is_output_label("prefs"));
    }

    #[test]
    fn three_recreations_per_minute() {
        let t0 = Instant::now();
        let mut h = VecDeque::new();
        let s = Duration::from_secs;
        assert!(allow_recreation(&mut h, t0));
        assert!(allow_recreation(&mut h, t0 + s(10)));
        assert!(allow_recreation(&mut h, t0 + s(20)));
        assert!(
            !allow_recreation(&mut h, t0 + s(30)),
            "a 4th within a minute"
        );
        assert!(!allow_recreation(&mut h, t0 + s(59)));
        // A minute after the first one, one more is allowed again.
        assert!(allow_recreation(&mut h, t0 + s(60)));
        assert!(!allow_recreation(&mut h, t0 + s(61)));
        assert!(allow_recreation(&mut h, t0 + s(200)));
        assert_eq!(h.len(), 1);
    }
}
