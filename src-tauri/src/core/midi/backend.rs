//! Where MIDI ports come from: the system (WinMM through `midir`) or, in tests and the e2e
//! fixture, an in-memory [`TestBackend`] (Windows has no virtual MIDI ports).
//!
//! Port names are what the driver reports (WinMM truncates them to 31 characters). Two ports
//! with the same name (two identical controllers) are told apart as `Name`, `Name #2`, … in
//! enumeration order.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

/// Called with each complete message of an open input, on the backend's own thread (WinMM's
/// callback thread for the system backend). Must not block.
pub type InputCallback = Box<dyn FnMut(&[u8]) + Send + 'static>;

/// An open input; dropping it closes the port.
pub type InputConn = Box<dyn Any + Send>;

/// The ports present now, by (unique) name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ports {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

/// An open output port.
pub trait MidiOut: Send {
    fn send(&mut self, msg: &[u8]) -> Result<(), String>;
}

/// A source of MIDI ports.
pub trait MidiBackend: Send + Sync {
    fn ports(&self) -> Ports;
    /// Opens an input. An error means it can't be used now (most often: another app has it).
    fn open_input(&self, name: &str, on_message: InputCallback) -> Result<InputConn, String>;
    fn open_output(&self, name: &str) -> Result<Box<dyn MidiOut>, String>;
    /// The system backend: input arrives on driver threads and is handed to the MIDI service
    /// thread, which also receives hot-plug notifications. Other backends deliver input inline.
    fn is_system(&self) -> bool {
        false
    }
}

/// `Name`, `Name #2`, … for repeated names, in order.
pub fn unique_names(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    names
        .into_iter()
        .map(|n| {
            let c = seen.entry(n.clone()).or_insert(0);
            *c += 1;
            if *c == 1 {
                n
            } else {
                format!("{n} #{c}")
            }
        })
        .collect()
}

// ---- System (WinMM via midir) ----------------------------------------------------------------

/// The machine's MIDI ports (WinMM on Windows).
pub struct SystemBackend;

const CLIENT: &str = "Kinetra";

/// WinMM handles may be used and closed from any thread; midir's connection types hold raw
/// handles, so they aren't `Send` by themselves.
struct Sendable<T>(T);
// SAFETY: a WinMM MIDI handle is a process-wide handle, valid on every thread; midir serializes
// access to the input handle with its own lock, and we only touch a connection through `&mut`
// (one thread at a time) or by dropping it.
unsafe impl<T> Send for Sendable<T> {}

impl MidiOut for Sendable<midir::MidiOutputConnection> {
    fn send(&mut self, msg: &[u8]) -> Result<(), String> {
        self.0.send(msg).map_err(|e| e.to_string())
    }
}

impl MidiBackend for SystemBackend {
    fn ports(&self) -> Ports {
        let inputs = midir::MidiInput::new(CLIENT)
            .map(|m| unique_names(m.ports().iter().filter_map(|p| m.port_name(p).ok())))
            .unwrap_or_default();
        let outputs = midir::MidiOutput::new(CLIENT)
            .map(|m| unique_names(m.ports().iter().filter_map(|p| m.port_name(p).ok())))
            .unwrap_or_default();
        Ports { inputs, outputs }
    }

    fn open_input(&self, name: &str, mut on_message: InputCallback) -> Result<InputConn, String> {
        let mut mi = midir::MidiInput::new(CLIENT).map_err(|e| e.to_string())?;
        // SysEx, clock and active sensing never reach the callback.
        mi.ignore(midir::Ignore::All);
        let ports = mi.ports();
        let names = unique_names(ports.iter().map(|p| mi.port_name(p).unwrap_or_default()));
        let i = names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| format!("MIDI input not found: {name}"))?;
        let conn = mi
            .connect(&ports[i], CLIENT, move |_, bytes, _| on_message(bytes), ())
            .map_err(|e| e.to_string())?;
        Ok(Box::new(Sendable(conn)))
    }

    fn open_output(&self, name: &str) -> Result<Box<dyn MidiOut>, String> {
        let mo = midir::MidiOutput::new(CLIENT).map_err(|e| e.to_string())?;
        let ports = mo.ports();
        let names = unique_names(ports.iter().map(|p| mo.port_name(p).unwrap_or_default()));
        let i = names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| format!("MIDI output not found: {name}"))?;
        let conn = mo.connect(&ports[i], CLIENT).map_err(|e| e.to_string())?;
        Ok(Box::new(Sendable(conn)))
    }

    fn is_system(&self) -> bool {
        true
    }
}

// ---- Test backend ----------------------------------------------------------------------------

/// A message sent to an output of the [`TestBackend`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SentMessage {
    pub output: String,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
struct TestInner {
    ports: Ports,
    /// Inputs and outputs that fail to open ("in use by another app").
    busy: HashSet<String>,
    open_inputs: HashMap<String, Arc<Mutex<InputCallback>>>,
    open_outputs: HashMap<String, usize>,
    sent: Vec<SentMessage>,
}

/// In-memory ports for tests and the e2e fixture: settable ports, inputs fed by
/// [`TestBackend::input`], and a record of everything sent to outputs.
#[derive(Default, Clone)]
pub struct TestBackend {
    inner: Arc<Mutex<TestInner>>,
}

impl TestBackend {
    pub fn new(inputs: &[&str], outputs: &[&str]) -> Arc<Self> {
        let b = Self::default();
        b.set_ports(
            inputs.iter().map(|s| s.to_string()).collect(),
            outputs.iter().map(|s| s.to_string()).collect(),
        );
        Arc::new(b)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TestInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the ports (call `AppState::midi_devices_changed` after, like a hot-plug).
    pub fn set_ports(&self, inputs: Vec<String>, outputs: Vec<String>) {
        self.lock().ports = Ports { inputs, outputs };
    }

    /// Makes a port fail to open (or open again).
    pub fn set_busy(&self, name: &str, busy: bool) {
        let mut g = self.lock();
        if busy {
            g.busy.insert(name.to_string());
        } else {
            g.busy.remove(name);
        }
    }

    /// Feeds a message into an open input (like the driver would). False when it isn't open.
    pub fn input(&self, name: &str, bytes: &[u8]) -> bool {
        // Called without the backend lock, so the callback may use the backend.
        let cb = self.lock().open_inputs.get(name).cloned();
        let Some(cb) = cb else { return false };
        let mut cb = cb.lock().unwrap_or_else(PoisonError::into_inner);
        (*cb)(bytes);
        true
    }

    /// Names of the open inputs, sorted.
    pub fn open_inputs(&self) -> Vec<String> {
        let mut v: Vec<String> = self.lock().open_inputs.keys().cloned().collect();
        v.sort();
        v
    }

    /// Names of the open outputs, sorted.
    pub fn open_outputs(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .lock()
            .open_outputs
            .iter()
            .filter(|(_, n)| **n > 0)
            .map(|(k, _)| k.clone())
            .collect();
        v.sort();
        v
    }

    /// Everything sent to outputs since the last call.
    pub fn take_sent(&self) -> Vec<SentMessage> {
        std::mem::take(&mut self.lock().sent)
    }
}

struct TestInput {
    backend: TestBackend,
    name: String,
}

impl Drop for TestInput {
    fn drop(&mut self) {
        self.backend.lock().open_inputs.remove(&self.name);
    }
}

struct TestOutput {
    backend: TestBackend,
    name: String,
}

impl MidiOut for TestOutput {
    fn send(&mut self, msg: &[u8]) -> Result<(), String> {
        self.backend.lock().sent.push(SentMessage {
            output: self.name.clone(),
            bytes: msg.to_vec(),
        });
        Ok(())
    }
}

impl Drop for TestOutput {
    fn drop(&mut self) {
        let mut g = self.backend.lock();
        if let Some(n) = g.open_outputs.get_mut(&self.name) {
            *n = n.saturating_sub(1);
        }
    }
}

impl MidiBackend for TestBackend {
    fn ports(&self) -> Ports {
        self.lock().ports.clone()
    }

    fn open_input(&self, name: &str, on_message: InputCallback) -> Result<InputConn, String> {
        let mut g = self.lock();
        if !g.ports.inputs.iter().any(|n| n == name) {
            return Err(format!("MIDI input not found: {name}"));
        }
        if g.busy.contains(name) {
            return Err("in use".into());
        }
        g.open_inputs
            .insert(name.to_string(), Arc::new(Mutex::new(on_message)));
        Ok(Box::new(TestInput {
            backend: self.clone(),
            name: name.to_string(),
        }))
    }

    fn open_output(&self, name: &str) -> Result<Box<dyn MidiOut>, String> {
        let mut g = self.lock();
        if !g.ports.outputs.iter().any(|n| n == name) {
            return Err(format!("MIDI output not found: {name}"));
        }
        if g.busy.contains(name) {
            return Err("in use".into());
        }
        *g.open_outputs.entry(name.to_string()).or_insert(0) += 1;
        Ok(Box::new(TestOutput {
            backend: self.clone(),
            name: name.to_string(),
        }))
    }
}
