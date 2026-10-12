//! MIDI control: devices, hot-plug, MIDI Learn, mappings, the `trigger.midi` flow trigger and LED
//! feedback.
//!
//! # In use
//! MIDI is **in use** while a project is open AND (it has a mapping, an armed deployed flow has a
//! `trigger.midi`, a learn session is open (`midi_learn`, counted) or the Preferences device
//! list watches (`watch_midi_devices`, counted)). Only then are ports open and the service
//! thread running. Otherwise nothing is open and no MIDI thread exists ([`AppState::midi_reconcile`]
//! decides; it runs on every change of those conditions and of `prefs.json` `midi`).
//!
//! # Threads (system backend)
//! - WinMM's own callback thread delivers input. The callback only parses nothing: it copies the
//!   bytes into a bounded queue (256 messages; more are dropped) and wakes the service thread
//!   with `PostMessage` (one of the few calls WinMM allows in a callback). It never blocks: a
//!   callback that blocks stalls the driver, and closing a port (`midiInReset`) waits for the
//!   callback, so running actions (screen lock, template reads) there could deadlock.
//! - ONE service thread ([`service`], Windows only): a message-only window that receives device
//!   notifications (`RegisterDeviceNotificationW`, every device interface class; debounced
//!   300 ms with a window timer) and the input wake-ups, and does the port work (enumerate, open,
//!   close). It runs actions in arrival order. It exists only while MIDI is in use and blocks in
//!   `GetMessageW` otherwise: no polling.
//!
//! Tests and the e2e fixture use [`backend::TestBackend`]: its input runs inline, port work
//! happens on the calling thread, and outputs are recorded. Without a backend (the default in
//! tests) MIDI does nothing.
//!
//! # Input
//! Note-on (velocity > 0), note-off and CC on channels 1–16 (`parse`). While a learn session is
//! open every note-on / CC is sent as `midi-learn` (≤ 20 per second) and nothing runs.
//! Otherwise:
//! - flows: `trigger.midi` fires on note-on and on a CC whose value changed;
//! - mappings: a compiled lookup by (kind, number), filtered by device / channel, swapped
//!   atomically when the mappings change (O(1) per message). One match runs through
//!   [`AppState::run_control`]: a note-on is `Press`, a note-off `Release`; a CC on a command is
//!   `Press` when it crosses 64 upwards (`Release` downwards; the last value per device / channel
//!   / number is kept, none counts as 0); a CC on a value is `Level(v / 127)`. Two or more
//!   matches run nothing and emit `control-conflict` (≤ 1 per 2 s per source).
//!
//! # LED feedback
//! Per input device (`prefs.json` `midi.feedback`): an output (the device's own by default) and a
//! profile ([`feedback`]). Mappings with `feedback: true` and a layer command (`live.*:<s>/<l>`)
//! show red on air, green when the layer exists off air, off otherwise; `cue.go:<flow>` shows red
//! while the flow runs, green while it is armed. Driven by the live hub's hook (every on-air
//! change, synchronously), flow-state transitions and reconciles; only changed LEDs are sent
//! (diffed against what each output was last sent). A newly opened output gets the full state;
//! closing a project (or MIDI going unused) switches every LED off first.

pub mod backend;
pub mod feedback;
pub mod parse;
#[cfg(windows)]
mod service;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{Duration, Instant};

use crate::core::control_actions::{parse_command, ControlCommand, ControlInput};
use crate::core::events::names;
use crate::core::model::{
    ControlTarget, ControlsDoc, MidiDevice, MidiFeedbackProfile, MidiKind, MidiLearned, MidiSource,
    TakeVia,
};
use crate::core::state::{AppState, StateChange};
use crate::error::{AppError, AppResult};

use backend::{InputConn, MidiBackend, MidiOut};
use feedback::{Led, LedKey};
use parse::MidiMsg;

/// Most queued input messages waiting for the service thread.
const QUEUE_MAX: usize = 256;
/// At most one `control-conflict` per source this often.
const CONFLICT_EVERY: Duration = Duration::from_secs(2);
/// At most one `midi-learn` this often (a moving fader sends many).
const LEARN_EVERY: Duration = Duration::from_millis(50);
/// Remembered CC values (per device / channel / number) before the table is reset.
const CC_MEMORY_MAX: usize = 4096;
/// Each step of the test pattern (red, then green, then the real state).
const TEST_STEP: Duration = Duration::from_millis(400);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- Compiled mappings ------------------------------------------------------------------------

/// One mapping in the lookup.
#[derive(Debug)]
struct Entry {
    id: String,
    device: Option<String>,
    channel: Option<u8>,
    target: ControlTarget,
}

/// What a feedback mapping's LED shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum FbWhat {
    Layer(String, String),
    Cue(String),
}

/// A mapping with LED feedback.
#[derive(Debug)]
struct FbEntry {
    device: Option<String>,
    key: LedKey,
    what: FbWhat,
}

/// The mappings compiled for the input path (see the module docs).
#[derive(Debug, Default)]
pub(crate) struct Lookup {
    map: HashMap<(MidiKind, u8), Vec<Entry>>,
    fb: Vec<FbEntry>,
    /// The layers feedback mappings show, once each.
    fb_layers: Vec<(String, String)>,
    /// LEDs per device filter, for the test pattern: every mapping's (device, key).
    all_keys: Vec<(Option<String>, LedKey)>,
    len: usize,
}

impl Lookup {
    pub(crate) fn build(doc: &ControlsDoc) -> Self {
        let mut l = Lookup {
            len: doc.mappings.len(),
            ..Default::default()
        };
        let mut layers = HashSet::new();
        for m in &doc.mappings {
            let s = &m.source;
            let key = (feedback::status(s.kind, s.channel), s.number);
            l.all_keys.push((s.device.clone(), key));
            l.map.entry((s.kind, s.number)).or_default().push(Entry {
                id: m.id.clone(),
                device: s.device.clone(),
                channel: s.channel,
                target: m.target.clone(),
            });
            if !m.feedback {
                continue;
            }
            let ControlTarget::Command { command_id } = &m.target else {
                continue;
            };
            let what = match parse_command(command_id) {
                Some(ControlCommand::CueGo { flow }) => FbWhat::Cue(flow.to_string()),
                Some(c) => match c.layer() {
                    Some((sc, la)) => FbWhat::Layer(sc.to_string(), la.to_string()),
                    None => continue,
                },
                None => continue,
            };
            if let FbWhat::Layer(sc, la) = &what {
                if layers.insert((sc.clone(), la.clone())) {
                    l.fb_layers.push((sc.clone(), la.clone()));
                }
            }
            l.fb.push(FbEntry {
                device: s.device.clone(),
                key,
                what,
            });
        }
        l
    }

    /// The mappings a message triggers.
    fn matching(&self, device: &str, channel: u8, kind: MidiKind, number: u8) -> Vec<&Entry> {
        self.map.get(&(kind, number)).map_or_else(Vec::new, |v| {
            v.iter()
                .filter(|e| {
                    e.channel.is_none_or(|c| c == channel)
                        && e.device.as_deref().is_none_or(|d| d == device)
                })
                .collect()
        })
    }

    fn cue_flows(&self) -> impl Iterator<Item = &str> {
        self.fb.iter().filter_map(|e| match &e.what {
            FbWhat::Cue(f) => Some(f.as_str()),
            FbWhat::Layer(..) => None,
        })
    }
}

// ---- Runtime state ----------------------------------------------------------------------------

/// Input waiting for the service thread (system backend).
#[derive(Default)]
pub(crate) struct InputQueue {
    items: Mutex<VecDeque<(Arc<str>, Vec<u8>)>>,
    /// The service window (0 = none yet).
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: std::sync::atomic::AtomicIsize,
    dropped: std::sync::atomic::AtomicU64,
}

impl InputQueue {
    /// Called on WinMM's callback thread: copy, queue, wake the service. Never blocks for long.
    fn push(&self, device: &Arc<str>, bytes: &[u8]) {
        let wake = {
            let mut q = lock(&self.items);
            if q.len() >= QUEUE_MAX {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
            q.push_back((device.clone(), bytes.to_vec()));
            q.len() == 1
        };
        if wake {
            #[cfg(windows)]
            service::post_input(self.hwnd.load(Ordering::Acquire));
        }
    }

    fn pop(&self) -> Option<(Arc<str>, Vec<u8>)> {
        lock(&self.items).pop_front()
    }
}

/// Open inputs and what the device list last said.
#[derive(Default)]
struct Ports {
    inputs: HashMap<String, InputConn>,
    /// Inputs that failed to open (in use by another app).
    busy: HashSet<String>,
    /// The list last emitted (`midi-devices`).
    devices: Vec<MidiDevice>,
}

/// An input device whose LEDs are driven: its output and profile.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Route {
    device: String,
    output: String,
    profile: MidiFeedbackProfile,
}

/// LED feedback state (see the module docs).
#[derive(Default)]
struct Feedback {
    outputs: HashMap<String, Box<dyn MidiOut>>,
    routes: Vec<Route>,
    /// What each output was last sent.
    last: HashMap<String, HashMap<LedKey, u8>>,
    /// Outputs showing the test pattern (live changes update `last` but aren't sent).
    testing: HashSet<String>,
    layers: HashMap<(String, String), Led>,
    running: HashSet<String>,
    armed: HashSet<String>,
}

impl Feedback {
    fn led(&self, what: &FbWhat) -> Led {
        match what {
            FbWhat::Layer(s, l) => self
                .layers
                .get(&(s.clone(), l.clone()))
                .copied()
                .unwrap_or(Led::Off),
            FbWhat::Cue(f) if self.running.contains(f) => Led::OnAir,
            FbWhat::Cue(f) if self.armed.contains(f) => Led::Ready,
            FbWhat::Cue(_) => Led::Off,
        }
    }

    /// What every open output should show.
    fn want(&self, lookup: &Lookup) -> HashMap<String, HashMap<LedKey, u8>> {
        let mut want: HashMap<String, HashMap<LedKey, u8>> = HashMap::new();
        for r in &self.routes {
            let w = want.entry(r.output.clone()).or_default();
            for e in &lookup.fb {
                if e.device.as_deref().is_none_or(|d| d == r.device) {
                    w.insert(e.key, feedback::velocity(r.profile, self.led(&e.what)));
                }
            }
        }
        want
    }

    /// Diffs every open output against `want` and sends the changes (except to outputs showing
    /// the test pattern). Returns the number of messages sent.
    fn apply(&mut self, lookup: &Lookup) -> usize {
        let want = self.want(lookup);
        let empty = HashMap::new();
        let mut sent = 0;
        let names: Vec<String> = self.outputs.keys().cloned().collect();
        for out in names {
            let last = self.last.entry(out.clone()).or_default();
            let msgs = feedback::diff(last, want.get(&out).unwrap_or(&empty));
            if self.testing.contains(&out) {
                continue;
            }
            if let Some(conn) = self.outputs.get_mut(&out) {
                for m in msgs {
                    if let Err(e) = conn.send(&m) {
                        tracing::debug!("MIDI feedback to {out}: {e}");
                    }
                    sent += 1;
                }
            }
        }
        sent
    }

    /// Switches every LED off and closes the outputs.
    fn close_all(&mut self, lookup: &Lookup) {
        self.routes.clear();
        self.testing.clear();
        self.apply(lookup);
        self.outputs.clear();
        self.last.clear();
    }
}

/// Input-path bookkeeping.
#[derive(Default)]
struct InputState {
    cc: HashMap<(Arc<str>, u8, u8), u8>,
    conflicts: HashMap<MidiSource, Instant>,
    learn_at: Option<Instant>,
}

/// MIDI runtime (one per `AppState`).
#[derive(Default)]
pub struct Midi {
    backend: RwLock<Option<Arc<dyn MidiBackend>>>,
    /// The service thread (system backend) and a counter of reconcile requests.
    svc: Mutex<Svc>,
    /// Serializes reconciles.
    reconcile: Mutex<()>,
    ports: Mutex<Ports>,
    lookup: RwLock<Arc<Lookup>>,
    queue: Arc<InputQueue>,
    input: Mutex<InputState>,
    learn: AtomicUsize,
    watch: AtomicUsize,
    fb: Mutex<Feedback>,
    /// Some output is open (cheap check for the live hook).
    fb_active: AtomicBool,
}

#[derive(Default)]
struct Svc {
    #[cfg(windows)]
    handle: Option<service::Service>,
    gen: u64,
}

impl Midi {
    fn backend(&self) -> Option<Arc<dyn MidiBackend>> {
        self.backend
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn lookup(&self) -> Arc<Lookup> {
        self.lookup
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether the service thread runs (tests and diagnostics).
    pub fn service_running(&self) -> bool {
        #[cfg(windows)]
        {
            lock(&self.svc).handle.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Input messages dropped because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.queue.dropped.load(Ordering::Relaxed)
    }
}

fn counted(c: &AtomicUsize, on: bool) {
    if on {
        c.fetch_add(1, Ordering::SeqCst);
    } else {
        // Never below zero (an unmatched close is ignored).
        let _ = c.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
    }
}

/// The device list: inputs (in port order), then outputs that aren't also inputs.
fn device_list(ports: &backend::Ports, busy: &HashSet<String>) -> Vec<MidiDevice> {
    let mut out: Vec<MidiDevice> = ports
        .inputs
        .iter()
        .map(|n| MidiDevice {
            name: n.clone(),
            input: true,
            output: ports.outputs.contains(n),
            active: !busy.contains(n),
        })
        .collect();
    for n in &ports.outputs {
        if !ports.inputs.contains(n) {
            out.push(MidiDevice {
                name: n.clone(),
                input: false,
                output: true,
                active: false,
            });
        }
    }
    out
}

impl AppState {
    /// Called once from `AppState::load`: keeps the compiled mappings in step with
    /// `controls.json`.
    pub(crate) fn midi_init(&self) {
        let weak = self.downgrade();
        self.on_controls_changed(Arc::new(move |doc: &Arc<ControlsDoc>| {
            let Some(st) = weak.upgrade() else { return };
            *st.midi_rt()
                .lookup
                .write()
                .unwrap_or_else(PoisonError::into_inner) = Arc::new(Lookup::build(doc));
            st.midi_reconcile();
        }));
        // LEDs follow what is on air and which cues run.
        self.add_change_listener(Arc::new(
            |st: &AppState, change: StateChange<'_>| match change {
                StateChange::Live => st.midi_live_changed(),
                StateChange::FlowRun { id, running } => st.midi_flow_state(id, running),
            },
        ));
    }

    /// Uses the machine's MIDI ports (the app calls this at startup; tests use
    /// [`AppState::set_midi_backend`] with a test backend, or nothing).
    pub fn use_system_midi(&self) {
        self.set_midi_backend(Some(Arc::new(backend::SystemBackend)));
    }

    /// Replaces the MIDI backend (`None`: MIDI off). Closes what the previous one had open.
    pub fn set_midi_backend(&self, backend: Option<Arc<dyn MidiBackend>>) {
        self.midi_close_all();
        *self
            .midi_rt()
            .backend
            .write()
            .unwrap_or_else(PoisonError::into_inner) = backend;
        self.midi_reconcile();
    }

    /// Whether MIDI is in use (see the module docs).
    fn midi_in_use(&self) -> bool {
        let m = self.midi_rt();
        self.project_dir().is_ok()
            && (m.lookup().len > 0
                || m.learn.load(Ordering::SeqCst) > 0
                || m.watch.load(Ordering::SeqCst) > 0
                || self.flow_engine().index().uses_midi())
    }

    /// Re-evaluates what should be open (see the module docs). Cheap when MIDI is unused and
    /// nothing is open. With the system backend the work happens on the service thread (started
    /// here when needed); otherwise inline.
    pub(crate) fn midi_reconcile(&self) {
        let m = self.midi_rt();
        let Some(b) = m.backend() else { return };
        if !b.is_system() {
            self.midi_reconcile_now();
            return;
        }
        #[cfg(windows)]
        {
            let mut svc = lock(&m.svc);
            svc.gen += 1;
            if let Some(h) = &svc.handle {
                h.reconcile();
                return;
            }
            if !self.midi_in_use() {
                return;
            }
            let weak = self.downgrade();
            match service::Service::start(m.queue.clone(), move |ev| {
                weak.upgrade().is_some_and(|st| st.midi_service_event(ev))
            }) {
                Ok(h) => svc.handle = Some(h),
                Err(e) => tracing::warn!("MIDI service thread: {e}"),
            }
        }
    }

    /// One event on the service thread. False: the thread ends (MIDI unused, everything closed).
    #[cfg(windows)]
    fn midi_service_event(&self, ev: service::Event) -> bool {
        match ev {
            service::Event::Input => {
                self.midi_drain();
                true
            }
            service::Event::Reconcile => {
                let gen = lock(&self.midi_rt().svc).gen;
                if self.midi_reconcile_now() {
                    return true;
                }
                let mut svc = lock(&self.midi_rt().svc);
                if svc.gen != gen {
                    // A request came in meanwhile; its message is queued for this thread.
                    return true;
                }
                // Dropped on its own thread: the handle doesn't join.
                svc.handle = None;
                false
            }
        }
    }

    /// Runs the queued input in order (service thread).
    #[cfg_attr(not(windows), allow(dead_code))]
    fn midi_drain(&self) {
        while let Some((device, bytes)) = self.midi_rt().queue.pop() {
            self.midi_handle(&device, &bytes);
        }
    }

    /// Opens and closes ports to match the in-use conditions, the ports present and the prefs;
    /// emits `midi-devices` when the list changed; refreshes the LEDs. Returns whether MIDI is
    /// in use.
    fn midi_reconcile_now(&self) -> bool {
        let m = self.midi_rt();
        let _g = lock(&m.reconcile);
        let Some(b) = m.backend() else {
            return false;
        };
        let in_use = self.midi_in_use();
        if !in_use {
            self.midi_close_ports();
            return false;
        }
        let prefs = self.prefs().midi;
        let present = b.ports();

        // Inputs.
        let mut ports = lock(&m.ports);
        let want: HashSet<&String> = present
            .inputs
            .iter()
            .filter(|n| prefs.input_enabled(n))
            .collect();
        ports.inputs.retain(|n, _| want.contains(n));
        ports.busy.retain(|n| want.contains(n));
        for name in &want {
            if ports.inputs.contains_key(*name) {
                continue;
            }
            let cb = self.midi_input_callback(&b, name);
            match b.open_input(name, cb) {
                Ok(conn) => {
                    ports.busy.remove(*name);
                    ports.inputs.insert((*name).clone(), conn);
                }
                Err(e) => {
                    tracing::info!("MIDI input {name} can't be opened: {e}");
                    ports.busy.insert((*name).clone());
                }
            }
        }
        let devices = device_list(&present, &ports.busy);
        let changed = ports.devices != devices;
        if changed {
            ports.devices = devices.clone();
        }
        drop(ports);
        if changed {
            self.midi_emit_devices(&devices);
        }

        // Feedback outputs.
        let routes: Vec<Route> = prefs
            .feedback
            .iter()
            .map(|(device, f)| Route {
                device: device.clone(),
                output: f.output.clone().unwrap_or_else(|| device.clone()),
                profile: f.profile,
            })
            .filter(|r| present.outputs.contains(&r.output))
            .collect();
        let needed: HashSet<String> = routes.iter().map(|r| r.output.clone()).collect();
        let lookup = m.lookup();
        let armed: HashSet<String> = lookup
            .cue_flows()
            .filter(|f| self.flow_engine().is_armed(f))
            .map(str::to_string)
            .collect();
        let opened: Vec<(String, Box<dyn MidiOut>)> = {
            let fb = lock(&m.fb);
            needed
                .iter()
                .filter(|o| !fb.outputs.contains_key(*o))
                .cloned()
                .collect::<Vec<_>>()
        }
        .into_iter()
        .filter_map(|o| match b.open_output(&o) {
            Ok(c) => Some((o, c)),
            Err(e) => {
                tracing::info!("MIDI output {o} can't be opened: {e}");
                None
            }
        })
        .collect();
        let layers = self.midi_layer_states(&lookup);
        let mut fb = lock(&m.fb);
        // Outputs no longer needed: LEDs off, then closed.
        let stale: Vec<String> = fb
            .outputs
            .keys()
            .filter(|o| !needed.contains(*o))
            .cloned()
            .collect();
        if !stale.is_empty() {
            fb.routes.retain(|r| !stale.contains(&r.output));
            fb.apply(&lookup);
            for o in &stale {
                fb.outputs.remove(o);
                fb.last.remove(o);
                fb.testing.remove(o);
            }
        }
        for (o, c) in opened {
            fb.last.remove(&o);
            fb.outputs.insert(o, c);
        }
        fb.routes = routes
            .into_iter()
            .filter(|r| fb.outputs.contains_key(&r.output))
            .collect();
        fb.armed = armed;
        fb.layers = layers;
        m.fb_active.store(!fb.outputs.is_empty(), Ordering::SeqCst);
        fb.apply(&lookup);
        true
    }

    /// Closes every port (LEDs off first). The device list keeps its last state.
    fn midi_close_ports(&self) {
        let m = self.midi_rt();
        let lookup = m.lookup();
        {
            let mut fb = lock(&m.fb);
            fb.close_all(&lookup);
            m.fb_active.store(false, Ordering::SeqCst);
        }
        let mut ports = lock(&m.ports);
        ports.inputs.clear();
        ports.busy.clear();
        lock(&m.input).cc.clear();
    }

    /// Closes everything now (backend change, app exit): LEDs off, ports closed, the service
    /// thread stopped.
    pub fn midi_close_all(&self) {
        let m = self.midi_rt();
        {
            let _g = lock(&m.reconcile);
            self.midi_close_ports();
        }
        #[cfg(windows)]
        {
            let h = lock(&m.svc).handle.take();
            drop(h);
        }
    }

    fn midi_input_callback(&self, b: &Arc<dyn MidiBackend>, name: &str) -> backend::InputCallback {
        let device: Arc<str> = Arc::from(name);
        if b.is_system() {
            // Driver thread: queue only (see the module docs).
            let queue = self.midi_rt().queue.clone();
            Box::new(move |bytes: &[u8]| queue.push(&device, bytes))
        } else {
            let weak = self.downgrade();
            Box::new(move |bytes: &[u8]| {
                if let Some(st) = weak.upgrade() {
                    st.midi_handle(&device, bytes);
                }
            })
        }
    }

    fn midi_emit_devices(&self, devices: &[MidiDevice]) {
        match serde_json::to_value(devices) {
            Ok(v) => self.events().emit(names::MIDI_DEVICES, v),
            Err(e) => tracing::warn!("midi-devices payload: {e}"),
        }
    }

    /// The device list: what the open ports say while MIDI is in use, otherwise a fresh
    /// enumeration (`active` is false only for an input that failed to open).
    pub fn midi_devices(&self) -> Vec<MidiDevice> {
        let m = self.midi_rt();
        let Some(b) = m.backend() else {
            return Vec::new();
        };
        if self.midi_in_use() {
            let ports = lock(&m.ports);
            if !ports.devices.is_empty() {
                return ports.devices.clone();
            }
        }
        device_list(&b.ports(), &HashSet::new())
    }

    /// A hot-plug notification (the service thread's, or a test's after changing the test
    /// backend's ports).
    pub fn midi_devices_changed(&self) {
        self.midi_reconcile();
    }

    /// Opens (`true`) or closes a learn session (counted).
    pub fn midi_learn(&self, on: bool) {
        counted(&self.midi_rt().learn, on);
        self.midi_reconcile();
    }

    /// The Preferences device list is shown (`true`) or hidden (counted): ports open and
    /// hot-plug is watched meanwhile.
    pub fn watch_midi_devices(&self, on: bool) {
        counted(&self.midi_rt().watch, on);
        self.midi_reconcile();
    }

    /// Test seam (tests, the e2e fixture, diagnostics): handles `bytes` as if they came from
    /// input `device`, synchronously on this thread. The device need not exist or be open.
    pub fn inject_midi(&self, device: &str, bytes: &[u8]) {
        self.midi_handle(device, bytes);
    }

    /// One message from an input (see the module docs).
    fn midi_handle(&self, device: &str, bytes: &[u8]) {
        let Some(msg) = parse::parse(bytes) else {
            return;
        };
        let m = self.midi_rt();
        let (kind, channel, number, value) = match msg {
            MidiMsg::NoteOn {
                channel,
                note,
                velocity,
            } => (MidiKind::Note, channel, note, velocity),
            MidiMsg::NoteOff { channel, note } => (MidiKind::Note, channel, note, 0),
            MidiMsg::Cc {
                channel,
                number,
                value,
            } => (MidiKind::Cc, channel, number, value),
        };
        // CC history (crossing 64, "changed") is kept also while learning.
        let prev_cc = if kind == MidiKind::Cc {
            let mut i = lock(&m.input);
            if i.cc.len() >= CC_MEMORY_MAX {
                i.cc.clear();
            }
            i.cc.insert((Arc::from(device), channel, number), value)
        } else {
            None
        };
        let starts = !matches!(msg, MidiMsg::NoteOff { .. });
        if m.learn.load(Ordering::SeqCst) > 0 {
            if starts {
                self.midi_learned(device, channel, kind, number, value);
            }
            return;
        }
        // Flows: note-on, or a CC whose value changed.
        let fires = match msg {
            MidiMsg::NoteOn { .. } => true,
            MidiMsg::Cc { .. } => prev_cc != Some(value),
            MidiMsg::NoteOff { .. } => false,
        };
        if fires {
            self.flow_midi(device, channel, kind, number, value);
        }
        // Mappings.
        let lookup = m.lookup();
        let hits = lookup.matching(device, channel, kind, number);
        let entry = match hits.as_slice() {
            [] => return,
            [one] => *one,
            many => {
                let ids: Vec<String> = many.iter().map(|e| e.id.clone()).collect();
                self.midi_conflict(device, channel, kind, number, ids);
                return;
            }
        };
        let input = match (&entry.target, msg) {
            (_, MidiMsg::NoteOn { .. }) => Some(ControlInput::Press),
            (_, MidiMsg::NoteOff { .. }) => Some(ControlInput::Release),
            (ControlTarget::Value { .. }, MidiMsg::Cc { value, .. }) => {
                Some(ControlInput::Level(f32::from(value) / 127.0))
            }
            (ControlTarget::Command { .. }, MidiMsg::Cc { value, .. }) => {
                cc_crossing(prev_cc.unwrap_or(0), value)
            }
        };
        let Some(input) = input else { return };
        if let Err(e) = self.run_control_via(&entry.target, input, TakeVia::Midi) {
            tracing::debug!("MIDI mapping {}: {e}", entry.id);
        }
    }

    fn midi_learned(&self, device: &str, channel: u8, kind: MidiKind, number: u8, value: u8) {
        {
            let mut i = lock(&self.midi_rt().input);
            let now = Instant::now();
            if i.learn_at
                .is_some_and(|t| now.duration_since(t) < LEARN_EVERY)
            {
                return;
            }
            i.learn_at = Some(now);
        }
        let l = MidiLearned {
            device: device.to_string(),
            channel,
            kind,
            number,
            value,
        };
        match serde_json::to_value(&l) {
            Ok(v) => self.events().emit(names::MIDI_LEARN, v),
            Err(e) => tracing::warn!("midi-learn payload: {e}"),
        }
    }

    fn midi_conflict(
        &self,
        device: &str,
        channel: u8,
        kind: MidiKind,
        number: u8,
        ids: Vec<String>,
    ) {
        let source = MidiSource {
            device: Some(device.to_string()),
            channel: Some(channel),
            kind,
            number,
        };
        {
            let mut i = lock(&self.midi_rt().input);
            let now = Instant::now();
            if i.conflicts.len() > 256 {
                i.conflicts
                    .retain(|_, t| now.duration_since(*t) < CONFLICT_EVERY);
            }
            if i.conflicts
                .get(&source)
                .is_some_and(|t| now.duration_since(*t) < CONFLICT_EVERY)
            {
                return;
            }
            i.conflicts.insert(source.clone(), now);
        }
        tracing::info!("MIDI {source:?} matches several mappings ({ids:?}); none ran");
        self.events().emit(
            names::CONTROL_CONFLICT,
            serde_json::json!({ "source": source, "mappingIds": ids }),
        );
    }

    // ---- Feedback ---------------------------------------------------------------------------

    fn midi_layer_states(&self, lookup: &Lookup) -> HashMap<(String, String), Led> {
        lookup
            .fb_layers
            .iter()
            .map(|(s, l)| {
                let led = match self.live().layer_view(s, l) {
                    Ok(v) if v.on_air => Led::OnAir,
                    Ok(_) => Led::Ready,
                    Err(_) => Led::Off,
                };
                ((s.clone(), l.clone()), led)
            })
            .collect()
    }

    /// The live hub's hook: what is on air may have changed. Sends changed LEDs only; nothing
    /// to do without a feedback output.
    pub(crate) fn midi_live_changed(&self) {
        let m = self.midi_rt();
        if !m.fb_active.load(Ordering::SeqCst) {
            return;
        }
        let lookup = m.lookup();
        if lookup.fb.is_empty() {
            return;
        }
        let layers = self.midi_layer_states(&lookup);
        let mut fb = lock(&m.fb);
        fb.layers = layers;
        fb.apply(&lookup);
    }

    /// A flow started (`running`) or ended (`flow-state`): cue LEDs. Touches only the feedback
    /// state (it runs under the flow engine's lock).
    pub(crate) fn midi_flow_state(&self, id: &str, running: bool) {
        let m = self.midi_rt();
        if !m.fb_active.load(Ordering::SeqCst) {
            return;
        }
        let lookup = m.lookup();
        let mut fb = lock(&m.fb);
        let changed = if running {
            fb.running.insert(id.to_string())
        } else {
            fb.running.remove(id)
        };
        if changed && lookup.cue_flows().any(|f| f == id) {
            fb.apply(&lookup);
        }
    }

    /// Lights every mapped pad of `device` red, then green, then shows the real state again
    /// (≈ 0.8 s, on a short-lived thread). `invalid` when the device has no LED feedback or no
    /// mapped pads, `notFound` when its feedback output isn't connected.
    pub fn test_midi_feedback(&self, device: &str) -> AppResult<()> {
        self.project_dir()?;
        let m = self.midi_rt();
        let Some(pref) = self.prefs().midi.feedback.get(device).cloned() else {
            return Err(AppError::Invalid(format!(
                "LED feedback is off for {device}"
            )));
        };
        let output = pref.output.unwrap_or_else(|| device.to_string());
        let lookup = m.lookup();
        let mut keys: Vec<LedKey> = lookup
            .all_keys
            .iter()
            .filter(|(d, _)| d.as_deref().is_none_or(|d| d == device))
            .map(|(_, k)| *k)
            .collect();
        keys.sort_unstable();
        keys.dedup();
        if keys.is_empty() {
            return Err(AppError::Invalid(format!(
                "No pads of {device} are mapped yet"
            )));
        }
        {
            let mut fb = lock(&m.fb);
            if !fb.outputs.contains_key(&output) {
                return Err(AppError::NotFound(format!("{output} is not connected")));
            }
            if !fb.testing.insert(output.clone()) {
                return Ok(());
            }
        }
        let weak = self.downgrade();
        let profile = pref.profile;
        let step = move |led: Option<Led>| -> bool {
            let Some(st) = weak.upgrade() else {
                return false;
            };
            let m = st.midi_rt();
            let mut fb = lock(&m.fb);
            let last = fb.last.get(&output).cloned().unwrap_or_default();
            if led.is_none() {
                fb.testing.remove(&output);
            }
            let Some(conn) = fb.outputs.get_mut(&output) else {
                fb.testing.remove(&output);
                return false;
            };
            for k in &keys {
                let v = match led {
                    Some(l) => feedback::velocity(profile, l),
                    None => last.get(k).copied().unwrap_or(0),
                };
                let _ = conn.send(&[k.0, k.1, v]);
            }
            true
        };
        std::thread::Builder::new()
            .name("midi-test-lights".into())
            .spawn(move || {
                for led in [Some(Led::OnAir), Some(Led::Ready)] {
                    if !step(led) {
                        return;
                    }
                    std::thread::sleep(TEST_STEP);
                }
                step(None);
            })
            .map_err(AppError::from)?;
        Ok(())
    }
}

/// A CC on a command mapping: `Press` when it crosses 64 upwards, `Release` downwards.
pub fn cc_crossing(prev: u8, value: u8) -> Option<ControlInput> {
    match (prev >= 64, value >= 64) {
        (false, true) => Some(ControlInput::Press),
        (true, false) => Some(ControlInput::Release),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
