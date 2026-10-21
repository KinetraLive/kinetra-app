//! Data shapes shared by `core`, the commands and the UI. Every type here is mirrored in
//! `src/lib/types.ts`; change both together. Field names serialize as camelCase.
//!
//! The on-disk files add a top-level `schemaVersion` around these (see `persist`); it is not part
//! of the in-memory structs.

use serde::{Deserialize, Serialize};

// ---- Project -------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Default for Resolution {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
        }
    }
}

/// A canary build (`KINETRA_CHANNEL=canary`, set by CI). Canary installs next to stable, so it
/// gets its own defaults where the two would collide (port, keychain service).
pub const CANARY_BUILD: bool = matches!(env!("KINETRA_CHANNEL").as_bytes(), b"canary");

/// Output server port of new projects: 7770, canary 7771.
pub const DEFAULT_PORT: u16 = if CANARY_BUILD { 7771 } else { 7770 };

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectSettings {
    pub port: u16,
    pub lan_access: bool,
    pub default_resolution: Resolution,
    /// The project's style (a style id from `list_styles`, i.e. a variant id of the library
    /// presets), or none. Set by `apply_style` with the project scope; layers added to a screen
    /// start on it when their template has that variant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Overrides colours, fonts and the logo of every template that declares them (see
    /// [`BrandKit`]). Omitted from the file when empty.
    #[serde(skip_serializing_if = "BrandKit::is_empty")]
    pub brand: BrandKit,
    /// OSC control (shown in Preferences → Server). Absent = off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub osc: Option<OscSettings>,
}

/// The project's brand kit. Variable precedence wherever variables resolve: layer override →
/// brand kit → the variant's value → the definition default. Each key applies only to templates
/// that declare the matching variable (`accent`, `secondary`, `text`, `plate`, `font-heading`,
/// `font-body`). Colours are `#rgb`/`#rrggbb`/`#rrggbbaa` (stored lowercase), fonts a
/// `TEMPLATE_FONTS` name. `logo` (`/assets/<name>`) fills every `logo` image field whose layer
/// value is empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrandKit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_heading: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
}

impl BrandKit {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The brand value for a template variable key, if the kit sets one.
    pub fn var(&self, key: &str) -> Option<&str> {
        match key {
            "accent" => self.accent.as_deref(),
            "secondary" => self.secondary.as_deref(),
            "text" => self.text.as_deref(),
            "plate" => self.plate.as_deref(),
            "font-heading" => self.font_heading.as_deref(),
            "font-body" => self.font_body.as_deref(),
            _ => None,
        }
    }
}

/// Shared variable keys of every library template (CSS `--<key>`). Mirrored in types.ts.
pub const STYLE_VARIABLES: &[&str] = &[
    "accent",
    "secondary",
    "text",
    "plate",
    "font-heading",
    "font-body",
    "radius",
    "speed",
];

/// A built-in design style (`presets/library.json`). Its id is the variant id in every library
/// preset that has the style (e.g. `frame`); `classic` is the look the older presets had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// Where `apply_style` applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ApplyStyleScope {
    /// Every layer of every screen; also sets `settings.style`.
    Project,
    /// One layer only (no settings change).
    Layer { screen_id: String, layer_id: String },
}

/// Result of `apply_style`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyStyleResult {
    /// Layers switched to the style (layers already on it are not counted).
    pub switched: u32,
    /// Layers whose template has no such variant and no library preset to copy it from.
    pub unchanged: u32,
    /// Distinct templates among `unchanged`.
    pub unchanged_templates: u32,
    /// Variants copied into project templates from their preset.
    pub added: u32,
    /// Token for `undo_apply_style` (only the latest one is valid); none when nothing switched
    /// and the project style did not change.
    pub undo: Option<String>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            lan_access: false,
            default_resolution: Resolution::default(),
            style: None,
            brand: BrandKit::default(),
            osc: None,
        }
    }
}

// ---- MIDI / OSC control ---------------------------------------------------------------------

/// Default OSC input port.
pub const OSC_DEFAULT_PORT: u16 = 9000;
/// Most entries in `OscSettings.allow`.
pub const OSC_ALLOW_CAP: usize = 32;
/// Longest OSC feedback host name.
pub const OSC_HOST_MAX: usize = 253;

/// OSC input on UDP `port` (1024–65535, default 9000). Messages from this computer are always
/// accepted; from other machines only when `ProjectSettings.lan_access` is on AND the sender's IP
/// is in `allow` (IPv4/IPv6 literals, ≤ 32). Optional feedback: on-air status sent to
/// `feedback` (host:port) as `/kinetra/layer/<screenId>/<layerId>/onair <0|1>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OscSettings {
    pub enabled: bool,
    pub port: u16,
    pub allow: Vec<String>,
    pub feedback: Option<OscFeedback>,
}

impl Default for OscSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: OSC_DEFAULT_PORT,
            allow: Vec::new(),
            feedback: None,
        }
    }
}

/// Where OSC feedback goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OscFeedback {
    pub host: String,
    pub port: u16,
}

/// The OSC listener (event `osc-status`, command `osc_status`): off without a project or with
/// OSC disabled; `listening` on UDP `port` (`lan`: on every interface, else 127.0.0.1 only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OscStatus {
    Off,
    Listening {
        port: u16,
        lan: bool,
    },
    Error {
        code: ServerErrorCode,
        port: u16,
        message: String,
    },
}

/// LED colour tables for MIDI feedback (on air = red, ready = green, off).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MidiFeedbackProfile {
    #[default]
    Generic,
    Apc40,
    ApcMini,
    Launchpad,
    Xtouch,
}

/// Longest MIDI device (port) name kept in prefs.
pub const MIDI_NAME_MAX: usize = 128;
/// Most devices in each table of `MidiPrefs`.
pub const MIDI_DEVICES_CAP: usize = 64;

/// Per machine (`prefs.json` `midi`), keyed by device (port) name.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MidiPrefs {
    /// Inputs that are listened to; a device not listed is on (new devices work at once).
    pub inputs: std::collections::BTreeMap<String, MidiInputPref>,
    /// LED feedback per input device: the output port (by name; `None` = the output with the
    /// input's name) and its profile. Absent = no feedback.
    pub feedback: std::collections::BTreeMap<String, MidiFeedbackPref>,
}

impl MidiPrefs {
    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty() && self.feedback.is_empty()
    }

    /// Whether input `device` is listened to (devices not listed are on).
    pub fn input_enabled(&self, device: &str) -> bool {
        self.inputs.get(device).is_none_or(|i| i.enabled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiInputPref {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiFeedbackPref {
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub profile: MidiFeedbackProfile,
}

/// A MIDI device as the core sees it (event `midi-devices`, command `list_midi_devices`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiDevice {
    pub name: String,
    pub input: bool,
    pub output: bool,
    /// Listening (input open).
    pub active: bool,
}

/// Note or control change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MidiKind {
    Note,
    Cc,
}

/// What a mapping listens to. `device` `None` = any device; `channel` `None` = any (1–16).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiSource {
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub channel: Option<u8>,
    pub kind: MidiKind,
    /// 0–127.
    pub number: u8,
}

impl MidiSource {
    /// Whether one MIDI message could trigger both `self` and `other` (same kind and number;
    /// devices and channels equal or either one "any").
    pub fn overlaps(&self, other: &MidiSource) -> bool {
        self.kind == other.kind
            && self.number == other.number
            && (self.channel.is_none() || other.channel.is_none() || self.channel == other.channel)
            && (self.device.is_none() || other.device.is_none() || self.device == other.device)
    }

    /// Whether a message from `device` on `channel` (1–16) with `kind` / `number` matches.
    pub fn matches(&self, device: &str, channel: u8, kind: MidiKind, number: u8) -> bool {
        self.kind == kind
            && self.number == number
            && self.channel.is_none_or(|c| c == channel)
            && self.device.as_deref().is_none_or(|d| d == device)
    }
}

/// The layer value a fader / knob sets (`ControlTarget::Value`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ValueTarget {
    /// Layer opacity, 0–100 %.
    Opacity,
    /// A `range` variable (min..max, step).
    Var { key: String },
    /// A `number` field.
    Field { key: String },
}

/// What a mapping does. `Command`: a registry command id the core understands
/// (`live.toggle:<s>/<l>`, `live.take:<s>/<l>`, `live.clear:<s>/<l>`, `live.update:<s>/<l>`,
/// `cue.go:<flowId>`, `source.next:<id>`, `source.prev:<id>`, `live.clearScreen:<s>`,
/// `live.clearAll`, `live.panic`); see `core::control_actions`. `Value`: a fader/knob on a
/// layer value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ControlTarget {
    Command {
        command_id: String,
    },
    Value {
        screen_id: String,
        layer_id: String,
        target: ValueTarget,
    },
}

/// One MIDI mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlMapping {
    /// nanoid(10).
    pub id: String,
    pub source: MidiSource,
    pub target: ControlTarget,
    /// Send LED status back for command mappings on layers/cues (on air / ready).
    #[serde(default)]
    pub feedback: bool,
}

/// Most mappings in `controls.json`.
pub const CONTROL_MAPPINGS_CAP: usize = 512;

/// `<project>/controls.json` (mappings follow the project). A source used by two mappings is a
/// conflict: both are kept and shown; the core runs neither and reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ControlsDoc {
    pub mappings: Vec<ControlMapping>,
}

/// One captured MIDI message while learning (event `midi-learn`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiLearned {
    pub device: String,
    pub channel: u8,
    pub kind: MidiKind,
    pub number: u8,
    pub value: u8,
}

/// Contents of `<project>/project.json` (minus `schemaVersion`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    pub name: String,
    /// RFC 3339.
    pub created_at: String,
    #[serde(default)]
    pub settings: ProjectSettings,
    /// The project's stable ID for remote control (a UUID v7; `register`'s `project_id`), written
    /// the first time remote control is turned on. A copied folder keeps it (a backup computer
    /// runs the same show); "Make this a separate show" gives the project a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The project's cloud backup ID (`app_project_id`, a UUID v7), written the first time it
    /// backs up. Unlike `id`, a copy gets its own (CLOUD_PROJECTS.md §2.4): a folder opened
    /// while the folder recorded for this ID still holds it is a copy (`core::cloud::projects`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_id: Option<String>,
}

/// An open project as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// Absolute path of the project directory.
    pub path: String,
    pub name: String,
    pub created_at: String,
    pub settings: ProjectSettings,
}

/// Result of inspecting a folder before anything is written to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FolderKind {
    Empty,
    Project {
        name: String,
    },
    /// `entries` is capped at `FOREIGN_ENTRY_CAP`; the UI shows "1000+" at the cap.
    NonEmptyForeign {
        entries: u32,
    },
}

pub const FOREIGN_ENTRY_CAP: u32 = 1000;

// ---- Recents -------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    /// RFC 3339.
    pub last_opened_at: String,
    /// Not meaningful on disk; filled in by the deferred existence check.
    #[serde(default)]
    pub missing: bool,
}

pub const RECENTS_CAP: usize = 10;

// ---- App preferences (app config dir, prefs.json) ------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackupPrefs {
    pub enabled: bool,
    pub interval_minutes: u32,
    pub retention: u32,
}

impl Default for BackupPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: 15,
            retention: 10,
        }
    }
}

pub const BACKUP_INTERVAL_RANGE: std::ops::RangeInclusive<u32> = 1..=240;
pub const BACKUP_RETENTION_RANGE: std::ops::RangeInclusive<u32> = 1..=100;

/// Simple hides Flows, the template code editor, MIDI/OSC and JSON API sources (the features keep
/// working in projects); Advanced shows everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UiMode {
    /// New installs (no prefs.json yet).
    #[default]
    Simple,
    Advanced,
}

impl UiMode {
    /// A prefs.json written before the mode existed: an existing install keeps every feature.
    fn existing_install() -> Self {
        UiMode::Advanced
    }
}

/// `Prefs.zoom` range and step (the WebView zoom of the app windows).
pub const ZOOM_RANGE: std::ops::RangeInclusive<f64> = 0.5..=2.0;
pub const ZOOM_STEP: f64 = 0.1;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub theme: Theme,
    pub backup: BackupPrefs,
    /// Always serialized. A missing prefs.json loads `Simple` (`Default`); a prefs.json without
    /// the key loads `Advanced` (an install from before the mode existed).
    #[serde(default = "UiMode::existing_install")]
    pub ui_mode: UiMode,
    /// WebView zoom of the app windows, within [`ZOOM_RANGE`] in steps of [`ZOOM_STEP`];
    /// `None` (omitted) = 1. `clamp_prefs` rounds it and turns 1 into `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f64>,
    /// Machine-wide key bindings: command id → keys (canonical spelling such as "Ctrl+Shift+P",
    /// "Shift+F3", "Escape"; `[]` = unbound). Overrides a command's default keys. Layer and cue
    /// keys are per project (`UiState.control.hotkeys`), never here. `clamp_prefs` drops invalid
    /// ids and keys and caps it at [`KEYBINDINGS_CAP`] entries of ≤ [`KEYS_PER_COMMAND`] keys.
    pub keybindings: std::collections::BTreeMap<String, Vec<String>>,
    /// MIDI devices on this machine (inputs on/off, LED feedback). Omitted when empty.
    #[serde(skip_serializing_if = "MidiPrefs::is_empty")]
    pub midi: MidiPrefs,
    /// Display outputs on this machine (≤ [`OUTPUTS_MAX`]). Omitted when empty. Entries that
    /// don't parse are dropped on load (one bad entry never loses the rest of prefs.json);
    /// `clamp_prefs` enforces the rest (see `core::outputs::clamp_outputs`).
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "crate::core::outputs::lenient_outputs"
    )]
    pub outputs: Vec<OutputConfig>,
    /// The local Control API on this machine (off by default). Omitted while off.
    #[serde(skip_serializing_if = "ControlApiPrefs::is_default")]
    pub control_api: ControlApiPrefs,
    /// Cloud backup on this machine. Omitted while default.
    #[serde(skip_serializing_if = "CloudPrefs::is_default")]
    pub cloud: CloudPrefs,
}

/// `Prefs.cloud`: cloud backup settings of this machine (`core::cloud::projects`). The server's
/// `client` defaults apply where these are `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CloudPrefs {
    /// Projects back up unless turned off one by one (owner decision 2026-10-15: on).
    pub new_projects: bool,
    /// While anything is on air: pause or limit transfers. `None` = the server's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_air: Option<CloudOnAir>,
    /// The limit for `limit`, in kbit/s ([`CLOUD_LIMIT_KBPS`]). `None` = the server's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_kbps: Option<u32>,
}

impl Default for CloudPrefs {
    fn default() -> Self {
        Self {
            new_projects: true,
            on_air: None,
            limit_kbps: None,
        }
    }
}

impl CloudPrefs {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `CloudPrefs.limitKbps` range.
pub const CLOUD_LIMIT_KBPS: std::ops::RangeInclusive<u32> = 64..=1_000_000;

pub use crate::core::cloud::projects::wire::OnAir as CloudOnAir;

// ---- Cloud backup (core::cloud::projects) -----------------------------------------------------

/// The account's cloud state from the last quota answer (CLOUD_PROJECTS.md §2.5, §4).
pub use crate::core::cloud::projects::wire::QuotaState as CloudState;

/// What the app may do with cloud backup on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudAccess {
    /// This build has no cloud backup (`KINETRA_CLOUD`, `CLOUD_BUILD`): nothing is asked, nothing
    /// runs, and the UI isn't in the build.
    Unavailable,
    /// Not signed in: nothing is asked.
    SignedOut,
    /// No `pro.cloud_sync` and no backups known on this computer: no request is made (the UI
    /// offers Pro, and "Check for backups" asks once).
    Free,
    /// The server is asked; `state` says what it allows.
    Available,
}

/// The quota as the UI shows it. Sizes in bytes (GB = 10⁹ for display), times RFC 3339.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudQuotaView {
    pub plan: Option<String>,
    pub used_bytes: u64,
    pub pending_bytes: u64,
    pub limit_bytes: u64,
    pub percent: u64,
    pub warn_percent: u64,
    pub projects_used: u64,
    pub projects_limit: u64,
    pub max_file_bytes: u64,
    pub retention_days: u64,
    pub kept_per_project: u64,
    pub upload_today_used: u64,
    pub upload_today_limit: u64,
    pub upload_today_resets_at: Option<String>,
    /// Lapsed: when everything is deleted.
    pub purge_at: Option<String>,
    /// After a downgrade: the shorter history (days) applies from `retention_from`.
    pub retention_days_next: Option<u64>,
    pub retention_from: Option<String>,
}

/// The cloud backup settings that apply (this machine's, else the server's defaults).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSettingsView {
    pub on_air: CloudOnAir,
    pub limit_kbps: u32,
    /// Seconds after the last change.
    pub interval_s: u64,
    pub new_projects: bool,
}

/// Cloud backup as the UI sees it (`cloud_view`, `cloud-changed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudView {
    pub access: CloudAccess,
    /// The last answer's state; `None` before the first one.
    pub state: Option<CloudState>,
    /// The last request didn't reach the server ("Cloud backup: offline, will retry.").
    pub offline: bool,
    /// Unix seconds of the last answer.
    pub checked_at: Option<i64>,
    pub quota: Option<CloudQuotaView>,
    pub settings: CloudSettingsView,
    /// The open project's backup (none without a project, or signed out).
    pub project: Option<CloudProjectView>,
    /// A restore running or just finished (one at a time; open project or not).
    pub restore: Option<CloudRestoreView>,
    /// Today's upload allowance is used: automatic backups continue at this time (RFC 3339).
    pub held_until: Option<String>,
}

/// How a version is restored (CLOUD_PROJECTS.md §10 "Restoring").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudRestoreMode {
    /// Into a new folder as a separate project (its own cloud backup).
    NewFolder,
    /// "In the cloud › Download": into a new folder, continuing the same cloud backup (unless
    /// this computer already has a folder for it; then it's a separate project).
    Download,
    /// Replace the open project's files with the version (backed up or copied first).
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudRestorePhase {
    Preparing,
    /// Replace: backing up the project as it is first.
    BackingUp,
    /// Replace: copying the project as it is first (when it can't back up).
    Copying,
    Downloading,
    Replacing,
}

/// What `cloud_restore_start` restores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRestoreRequest {
    /// The server's project id.
    pub project_id: String,
    /// The version; `None` = the newest.
    pub version_id: Option<String>,
    pub mode: CloudRestoreMode,
    /// The new folder (new folder, download): must not exist yet, or be empty.
    pub folder: Option<String>,
}

/// What a restore will do, for the dialog (`cloud_restore_plan`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRestorePlan {
    /// The suggested new folder (new folder, download); the open project's (replace).
    pub folder: String,
    /// Replace: the project is backed up first (`true`), else copied to `copyFolder`.
    pub backup_first: bool,
    pub copy_folder: Option<String>,
}

/// A restore in progress, or how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRestoreView {
    pub mode: CloudRestoreMode,
    pub version_id: String,
    /// The folder restored into (the open project's for replace).
    pub folder: String,
    pub phase: CloudRestorePhase,
    pub done_bytes: u64,
    pub total_bytes: u64,
    /// Waiting because something is on air.
    pub paused: bool,
    pub limited: bool,
    /// Set once it ended.
    pub result: Option<CloudRestoreResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRestoreResult {
    /// The folder holds the version (damaged files aside, new folder only).
    pub ok: bool,
    /// Why it stopped, in plain words (none when it worked).
    pub error: Option<String>,
    /// Files that didn't arrive intact after a retry (at most [`CLOUD_REPORT_MAX`]).
    pub damaged: Vec<String>,
    pub damaged_count: u32,
    /// Replace: where the copy of the project as it was went (when it wasn't backed up).
    pub copy_folder: Option<String>,
    /// Files taken from this computer instead of downloaded.
    pub reused: u32,
}

/// The open project's cloud backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudProjectView {
    /// Backup is on for this project (its own switch, else `CloudPrefs.newProjects`).
    pub backup: bool,
    /// The server knows this project (registered from this computer).
    pub registered: bool,
    /// The server's id for it (restores of the open project name it).
    pub server_id: Option<String>,
    /// The one-time "Backing up to the cloud" notice was shown.
    pub notice_seen: bool,
    /// A choice waiting for the user (shown quietly, never a modal).
    pub question: Option<CloudQuestion>,
    /// Why this project can't back up now (e.g. the plan's project limit), in plain words.
    pub issue: Option<String>,
    /// A backup running now.
    pub run: Option<CloudRunView>,
    /// RFC 3339 of the last backup that finished (this computer).
    pub last_backup_at: Option<String>,
    /// Files the last backup left out, and why (at most [`CLOUD_REPORT_MAX`]).
    pub report: Vec<CloudSkipped>,
    /// How many files the last backup left out in all.
    pub skipped: u32,
    /// Why the last backup didn't finish, in plain words ("Cloud backup: offline, will retry.").
    pub error: Option<String>,
    /// The last backup found another computer's newer backup (both kept, §9).
    pub conflict: Option<CloudConflictView>,
}

/// A version in the history (CLOUD_PROJECTS.md §7, §10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudVersionView {
    pub id: String,
    /// RFC 3339.
    pub created_at: String,
    /// The computer it was backed up from.
    pub machine: Option<String>,
    pub label: Option<String>,
    pub kept: bool,
    /// The newest version (can't be deleted).
    pub is_head: bool,
    /// The server is missing some of its files: it can't be fully restored.
    pub damaged: bool,
    /// Part of a conflict not dismissed yet.
    pub conflict: bool,
    pub file_count: u64,
    /// The files' total size; `new_bytes` = what it stored for the first time.
    pub logical_bytes: u64,
    pub new_bytes: u64,
    /// auto | close | manual.
    pub reason: Option<String>,
    /// When retention deletes it (none for the newest, kept and conflicting versions).
    pub expires_at: Option<String>,
}

/// One page of the history, newest first; `next` asks for the page after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudVersionPage {
    pub versions: Vec<CloudVersionView>,
    pub next: Option<String>,
}

/// A project in the account's cloud backup (Preferences › Cloud backup › Cloud projects).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudProjectSummary {
    pub id: String,
    pub name: String,
    pub versions: u64,
    pub kept: u64,
    pub conflict: bool,
    /// The newest version's time and computer.
    pub last_at: Option<String>,
    pub last_machine: Option<String>,
    /// In the trash: deleted for good on `purge_at`.
    pub trashed_at: Option<String>,
    pub purge_at: Option<String>,
    /// The open project's backup.
    pub open: bool,
    /// This computer has a folder for it.
    pub here: bool,
}

/// Files listed in a backup report at most.
pub const CLOUD_REPORT_MAX: usize = 100;

/// A backup in progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRunView {
    pub phase: CloudPhase,
    /// Bytes read (hashing) or sent (uploading) so far, of `total_bytes`.
    pub done_bytes: u64,
    pub total_bytes: u64,
    /// Waiting because something is on air.
    pub paused: bool,
    /// Sending slowly because something is on air (`limit`).
    pub limited: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudPhase {
    Scanning,
    Hashing,
    Uploading,
    Committing,
}

/// A file a backup left out.
pub use crate::core::cloud::projects::scan::Skipped as CloudSkipped;

/// Another computer backed this project up in between (CLOUD_PROJECTS.md §9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudConflictView {
    pub machine: Option<String>,
    pub at: Option<String>,
    /// The version that has the conflict (Dismiss resolves it).
    pub version_id: String,
    /// The other computer's version ("Open the other version").
    pub other_version_id: Option<String>,
}

/// A choice cloud backup needs from the user (CLOUD_PROJECTS.md §5, the app plan's decision 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CloudQuestion {
    /// The project's backup is in the trash: Restore / Start a new cloud backup / Turn off.
    #[serde(rename_all = "camelCase")]
    InTrash { purge_at: Option<String> },
    /// The first automatic backup would upload `bytes` (over 1 GB): Back up / Turn off.
    #[serde(rename_all = "camelCase")]
    FirstBackup { bytes: u64 },
    /// Another computer already backs this project up and this one has no record of it:
    /// Continue that backup here / Back up as a separate project / Don't back up here.
    #[serde(rename_all = "camelCase")]
    OtherComputer {
        machine: Option<String>,
        at: Option<String>,
    },
}

/// The user's answer to a [`CloudQuestion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudAnswer {
    /// A large first backup: go ahead.
    BackUp,
    /// In the trash: restore it and carry on.
    Restore,
    /// In the trash: leave it there and start a new cloud backup.
    StartNew,
    /// Another computer's backup: carry on from it here.
    ContinueHere,
    /// Another computer's backup: give this project its own.
    Separate,
    /// Either: turn backup off for this project.
    TurnOff,
}

/// `Prefs.controlApi`: the local Control API (`core::control_api`). The token lives in the
/// keychain, never here.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ControlApiPrefs {
    pub enabled: bool,
}

impl ControlApiPrefs {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

// ---- Display outputs (per machine, prefs.json `outputs`) -----------------------------------

/// A connected display as the OS reports it (`list_displays`, `displays-changed`). Physical
/// pixels in the virtual desktop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    /// The monitor's device interface path (stable across reboots and replugging into the same
    /// port).
    pub id: String,
    /// The EDID's monitor name ("DELL U2720Q"); else the device description.
    pub name: String,
    /// EDID manufacturer + product + serial (`DEL-A0B1-12345678`); empty when unreadable.
    pub edid_key: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub primary: bool,
    /// Kinetra's main window is on this display.
    pub has_main_window: bool,
}

/// What a fill output shows behind the screen.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OutputBackground {
    #[default]
    Black,
    /// [`CHROMA_GREEN`].
    Green,
    /// `#rrggbb` (lowercase after `clamp_prefs`).
    Color { color: String },
}

/// "Fill on green" (chroma).
pub const CHROMA_GREEN: &str = "#00b140";
/// Most outputs per machine.
pub const OUTPUTS_MAX: usize = 8;
/// Longest output name (characters).
pub const OUTPUT_NAME_MAX: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputKind {
    /// A borderless window on a connected display.
    #[default]
    Display,
    /// An NDI® source on the network (`OutputConfig.ndi`; Windows). Its source name is the
    /// output's name.
    Ndi,
}

/// The frame rates an NDI output can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NdiRate {
    #[serde(rename = "25")]
    R25,
    #[serde(rename = "29.97")]
    R29_97,
    #[serde(rename = "30")]
    R30,
    #[default]
    #[serde(rename = "50")]
    R50,
    #[serde(rename = "59.94")]
    R59_94,
    #[serde(rename = "60")]
    R60,
}

/// Which bus an NDI output sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NdiBus {
    #[default]
    Program,
    /// Studio mode's Preview (Advanced).
    Preview,
}

/// An NDI output's settings (see `types.ts` `NdiOutputSettings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NdiOutputSettings {
    #[serde(default)]
    pub rate: NdiRate,
    /// Send transparency (UYVA); off = composited over `OutputConfig.background` (UYVY).
    #[serde(default = "default_true")]
    pub alpha: bool,
    /// Color multiplied by alpha (Advanced; some receivers want it). Off = straight alpha.
    #[serde(default)]
    pub premultiplied: bool,
    /// NDI groups, comma-separated (Advanced); `None` = NDI's default group.
    #[serde(default)]
    pub groups: Option<String>,
    #[serde(default)]
    pub bus: NdiBus,
}

impl Default for NdiOutputSettings {
    fn default() -> Self {
        Self {
            rate: NdiRate::default(),
            alpha: true,
            premultiplied: false,
            groups: None,
            bus: NdiBus::Program,
        }
    }
}

/// Most NDI outputs per machine (within [`OUTPUTS_MAX`]).
pub const NDI_OUTPUTS_MAX: usize = 4;
/// Longest NDI groups text.
pub const NDI_GROUPS_MAX: usize = 128;

/// What an NDI output's receivers do right now (in [`OutputStatus::ndi`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NdiLive {
    /// NDI connections to the source.
    pub receivers: u32,
    /// A receiver has it on program (tally).
    pub program: bool,
    /// A receiver has it on preview (tally).
    pub preview: bool,
    /// More than 1 % of frames were dropped in the last second.
    pub dropping: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputMode {
    #[default]
    Fill,
    Key,
}

/// The display an output is assigned to, remembered for the "disconnected" warning and the
/// fallback match (EDID key + size) when it comes back on another port.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputDisplayRef {
    pub id: String,
    pub name: String,
    pub edid_key: String,
    pub width: u32,
    pub height: u32,
}

/// One display output (see `types.ts` `OutputConfig`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputConfig {
    pub id: String,
    #[serde(default)]
    pub kind: OutputKind,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub display: OutputDisplayRef,
    /// A screen of the open project (key outputs use their fill's).
    #[serde(default)]
    pub screen_id: Option<String>,
    #[serde(default)]
    pub mode: OutputMode,
    /// Fill outputs only (key outputs are always black).
    #[serde(default)]
    pub background: OutputBackground,
    /// Key outputs: the fill output it keys for. `None` for fill outputs.
    #[serde(default)]
    pub key_of: Option<String>,
    #[serde(default)]
    pub always_on_top: bool,
    /// Media audio (fill outputs; key outputs are always silent). `None` = on, to the system
    /// default device. Clamped by `outputs::clamp_outputs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<OutputAudio>,
    /// NDI outputs' settings (`Some` for `kind: ndi` after `clamp_outputs`, else `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ndi: Option<NdiOutputSettings>,
}

/// Where an output's media audio plays (see `types.ts` `OutputConfig.audio`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputAudio {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// The audio output's label as the OS shows it; `None` = the system default.
    #[serde(default)]
    pub device: Option<String>,
}

/// Longest stored audio device label.
pub const OUTPUT_AUDIO_DEVICE_MAX: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputState {
    Off,
    Showing,
    DisplayMissing,
    NoProject,
    ScreenMissing,
    Failed,
    /// An NDI output, but the NDI Runtime isn't installed.
    NdiMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TestPattern {
    Bars,
    Grid,
    Safe,
    Name,
}

impl TestPattern {
    pub fn as_str(self) -> &'static str {
        match self {
            TestPattern::Bars => "bars",
            TestPattern::Grid => "grid",
            TestPattern::Safe => "safe",
            TestPattern::Name => "name",
        }
    }
}

/// Live status per output (`list_outputs`, `outputs-changed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputStatus {
    pub id: String,
    pub state: OutputState,
    pub display_id: Option<String>,
    pub frame_locked: Option<bool>,
    pub pattern: Option<TestPattern>,
    pub message: Option<String>,
    /// NDI outputs that are sending: receivers and tally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ndi: Option<NdiLive>,
}

// `zoom` is the only float: JSON has no NaN and `clamp_prefs` keeps it finite, so equality is
// reflexive.
impl Eq for Prefs {}

/// Most entries in `Prefs.keybindings`.
pub const KEYBINDINGS_CAP: usize = 500;
/// Most keys bound to one command.
pub const KEYS_PER_COMMAND: usize = 4;
/// Longest command id (bindings, the palette's Recent).
pub const COMMAND_ID_MAX: usize = 200;
/// Commands the palette remembers per project (`UiState.palette.recent`).
pub const PALETTE_RECENT_CAP: usize = 5;

// ---- Per-project UI state (<project>/ui-state.json) ----------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ControlView {
    #[default]
    Grid,
    Single,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LiveEdit {
    /// Field edits go on air as you type.
    Immediate,
    /// Field edits wait for the next TAKE.
    #[default]
    OnTake,
}

/// Control-mode layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ControlViewState {
    pub view: ControlView,
    pub grid_columns: u32,
    pub selected_screen_id: Option<String>,
    pub hidden_screen_ids: Vec<String>,
    pub live_edit: LiveEdit,
    /// `"<screenId>/<layerId>"` to key combo: `F1`–`F12`, optionally `Shift+` (e.g. `"Shift+F3"`).
    /// The key toggles the layer (TAKE when off air, CLEAR when on air) while the Control page is
    /// focused. Invalid and duplicate combos are dropped by `clamp_ui_state` (the last one wins).
    pub hotkeys: std::collections::BTreeMap<String, String>,
    /// Automation off (next to PANIC): no automatic trigger fires, auto-advance holds, a
    /// countdown with `end: hide` stays on air. Cues and hotkeys still run. Persisted so a
    /// restart mid-show keeps it off.
    pub automation_off: bool,
    /// Width of Control's right panel in px (dragged; clamped to `CONTROL_PANEL_WIDTH_RANGE`);
    /// `None` = the default width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_width: Option<u32>,
    /// Studio mode (Preview/Program). Absent in older files = off. Changed only with
    /// `set_studio` (`set_ui_state` keeps the stored value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub studio: Option<StudioSettings>,
    /// Server-owned (not in the UI's types): the Studio Preview's visibility cues per screen
    /// (`screenId → layerId → visible`), so a restart keeps the Preview. Staged content needs no
    /// copy here: it is saved in the screen files. `set_ui_state` keeps the stored value.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub studio_preview: StudioCues,
}

/// Studio visibility cues: `screenId → layerId → visible`.
pub type StudioCues = std::collections::BTreeMap<String, std::collections::BTreeMap<String, bool>>;

/// After a take: what Preview becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewAfterTake {
    /// Every layer cued out in Preview (a blank slate).
    Clear,
    /// Preview mirrors the new Program (empty diff).
    #[default]
    Keep,
}

/// Studio mode settings, per project (`ui-state.json`, `control.studio`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StudioSettings {
    pub enabled: bool,
    pub preview_after_take: PreviewAfterTake,
    pub transition: TakeTransition,
}

impl StudioSettings {
    /// The transition's `ms` clamped into range.
    pub fn clamped(mut self) -> Self {
        self.transition = self.transition.clamped();
        self
    }
}

/// How `set_studio` turns Studio mode off when Preview has changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StudioExit {
    /// Take Preview to Program first (the default transition).
    Take,
    /// Drop the Preview diff.
    Discard,
    /// Staged content of on-air layers becomes "Changes on next take" (`onTake` staging);
    /// staged content of off-air layers applies now; cues are dropped.
    KeepStaged,
}

/// The one studio operation entry point (`AppState::apply_studio_op`): every take and cue from
/// Control, hotkeys, the palette, Flows and later REST goes through it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum StudioOp {
    /// Cue a layer in (`true`) or out (`false`) of Preview; `null` = as Program.
    Cue {
        layer_id: String,
        #[serde(default)]
        visible: Option<bool>,
    },
    /// Preview → Program with `transition` (`null` = the project default).
    Take {
        #[serde(default)]
        transition: Option<TakeTransition>,
    },
    TbarBegin {
        #[serde(default)]
        transition: Option<TakeTransition>,
    },
    /// T-bar position, clamped into 0..=1.
    TbarSet { p: f64 },
    /// `commit`: released at the end (= take); otherwise abort (Program unchanged).
    TbarEnd { commit: bool },
    /// Empty the diff: Preview = Program.
    Clear {},
}

impl Default for ControlViewState {
    fn default() -> Self {
        Self {
            view: ControlView::Grid,
            grid_columns: 2,
            selected_screen_id: None,
            hidden_screen_ids: Vec::new(),
            live_edit: LiveEdit::OnTake,
            hotkeys: Default::default(),
            automation_off: false,
            panel_width: None,
            studio: None,
            studio_preview: Default::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiState {
    /// Sidebar pinned open; otherwise an icon rail that expands on hover. Replaces the earlier
    /// `sidebarCollapsed`, which old files may still contain (unknown fields are ignored).
    pub sidebar_pinned: bool,
    pub control: ControlViewState,
    /// The screen layout editor's view options (all screens of the project).
    pub screen_editor: ScreenEditorView,
    /// The command palette's per-project state.
    pub palette: PaletteState,
}

/// The command palette's per-project state. `clamp_ui_state` keeps ≤ [`PALETTE_RECENT_CAP`]
/// valid, distinct ids.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PaletteState {
    /// The last commands run from the palette, newest first.
    pub recent: Vec<String>,
}

/// Control's right panel width (px), when the user has dragged it.
pub const CONTROL_PANEL_WIDTH_RANGE: std::ops::RangeInclusive<u32> = 280..=640;
/// Grid sizes the layout editor accepts (screen px).
pub const SCREEN_GRID_SIZE_RANGE: std::ops::RangeInclusive<u32> = 2..=512;
/// Aspect crop overlays of the layout editor; anything else is stored as `"none"`.
pub const SCREEN_CROP_OVERLAYS: &[&str] = &["none", "9:16", "4:5", "1:1", "4:3"];

/// View options of the screen layout editor. Editor-only: nothing here reaches the output.
/// `clamp_ui_state` clamps `grid_size` into [`SCREEN_GRID_SIZE_RANGE`] and turns an unknown
/// `crop` into `"none"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScreenEditorView {
    /// Boxes snap while dragging (Alt disables it for one drag).
    pub snap: bool,
    /// Ruler guides shown (and snapped to).
    pub guides: bool,
    pub grid: bool,
    /// Grid spacing in screen px.
    pub grid_size: u32,
    /// Action safe: the central 93 % (SMPTE ST 2046-1).
    pub action_safe: bool,
    /// Title safe: the central 90 % (SMPTE ST 2046-1).
    pub title_safe: bool,
    pub center_cross: bool,
    /// The screen's own named safe areas (`ScreenMeta.safe_areas`).
    pub custom_safe: bool,
    /// Aspect crop overlay: one of [`SCREEN_CROP_OVERLAYS`].
    pub crop: String,
}

impl Default for ScreenEditorView {
    fn default() -> Self {
        Self {
            snap: true,
            guides: true,
            grid: false,
            grid_size: 16,
            action_safe: false,
            title_safe: false,
            center_cross: false,
            custom_safe: true,
            crop: "none".into(),
        }
    }
}

// ---- Backups ------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackupReason {
    Open,
    Timer,
    Manual,
    PreImport,
    PreMove,
    PreRestore,
    PreReset,
    /// Before migrating rules and macros to flows.
    PreMigrate,
    /// Before a template was updated to its preset's latest look.
    PreUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    /// Directory name under `.backups/`, e.g. `20261003T171200Z-manual`.
    pub id: String,
    pub created_at: String,
    pub reason: BackupReason,
    pub files: u32,
    pub bytes: u64,
}

// ---- Settings export/import ---------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsSection {
    Prefs,
    Project,
    UiState,
    /// The project's MIDI mappings (`controls.json`).
    Controls,
}

pub const SETTINGS_EXPORT_KIND: &str = "kinetra/settings";
/// Kinds written by earlier versions (the app was called "Broadcast"); still importable.
pub const LEGACY_SETTINGS_EXPORT_KINDS: &[&str] = &["broadcast-app/settings"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldChange {
    /// Dotted path, e.g. `backup.intervalMinutes`.
    pub key: String,
    pub from: serde_json::Value,
    pub to: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionDiff {
    pub section: SettingsSection,
    /// Empty when the section is present but identical to the current state.
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub exported_at: String,
    pub app_version: String,
    /// Only sections present in the file. `project`/`uiState` are omitted when no project is open.
    pub sections: Vec<SectionDiff>,
}

// ---- Move project -------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MovePhase {
    Scanning,
    Snapshot,
    Copying,
    Verifying,
    Switching,
    Deleting,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveProgress {
    pub phase: MovePhase,
    pub files_done: u32,
    pub files_total: u32,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

// ---- Reset --------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResetScope {
    /// Theme only.
    Appearance,
    /// `UiState.control` of the open project.
    ControlLayout,
    /// All of `Prefs`, plus the open project's `UiState`.
    Preferences,
    /// Preferences + recents + the open project's settings; closes the project.
    Everything,
}

// ---- App info -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub tauri_version: String,
    pub webview_version: String,
    pub config_dir: String,
    pub log_dir: String,
    /// Build channel (`KINETRA_CHANNEL` at build time, default stable). Picks the update feed.
    pub channel: BuildChannel,
    /// Short git commit of the build ("unknown" outside a git checkout).
    pub commit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuildChannel {
    Stable,
    Canary,
}

impl BuildChannel {
    /// Parses the build-time channel name (build.rs accepts only these two).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "stable" => Some(BuildChannel::Stable),
            "canary" => Some(BuildChannel::Canary),
            _ => None,
        }
    }
}

// ---- Native menu (set_app_menu) -------------------------------------------------------------

/// The app menu, built by the frontend from the command registry (see `src/lib/types.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuSpec {
    pub menus: Vec<MenuSubmenu>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum MenuNode {
    Item(MenuItemSpec),
    Submenu(MenuSubmenu),
    Separator,
    Predefined { item: MenuPredefined },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuItemSpec {
    /// Registry command id; unique in the spec.
    pub id: String,
    /// Translated title ("&" mnemonics allowed).
    pub text: String,
    /// Display keys in canonical spelling ("Ctrl+Shift+P"); `None` = none.
    #[serde(default)]
    pub keys: Option<String>,
    pub enabled: bool,
    /// A check state; `None` = a plain item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuSubmenu {
    pub id: String,
    pub text: String,
    pub enabled: bool,
    pub items: Vec<MenuNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MenuPredefined {
    Cut,
    Copy,
    Paste,
    SelectAll,
    Undo,
    Redo,
}

// ---- Updates ---------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// A newer version exists on this build's channel.
    pub available: bool,
    pub version: Option<String>,
    /// Release notes (plain text) and date (RFC 3339), when the feed has them.
    pub notes: Option<String>,
    pub date: Option<String>,
    /// Why Install is disabled right now (anything on air), or `None`.
    pub blocked: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePhase {
    Download,
    Install,
}

/// `update-progress` while installing (≤ 10 per second).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub phase: UpdatePhase,
}

// ---- Output server --------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerErrorCode {
    PortInUse,
    BindFailed,
}

/// State of the output server, which follows the open project (stopped when none is open).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ServerStatus {
    Stopped,
    Starting {
        port: u16,
    },
    Running {
        port: u16,
        /// Base URL on this machine, e.g. `http://127.0.0.1:7770`.
        url: String,
        /// Base URL for other machines when LAN access is on and a LAN address was found.
        lan_url: Option<String>,
    },
    Error {
        code: ServerErrorCode,
        port: u16,
        message: String,
    },
}

// ---- Templates (templates/<id>/template.json + template.html + template.css) ----------------

/// Fonts templates may use. Served by the output server at `/fonts/<slug>.css`, so every machine
/// renders the same. A `font` variable's value is one of these names; the CSS value is the stack.
/// (name, slug, CSS font-family stack). Mirrored in `TEMPLATE_FONTS` in types.ts.
pub const TEMPLATE_FONTS: &[(&str, &str, &str)] = &[
    ("Barlow", "barlow", "\"Barlow\", system-ui, sans-serif"),
    (
        "Barlow Condensed",
        "barlow-condensed",
        "\"Barlow Condensed\", \"Arial Narrow\", sans-serif",
    ),
    (
        "Oswald",
        "oswald",
        "\"Oswald\", \"Arial Narrow\", sans-serif",
    ),
    (
        "Montserrat",
        "montserrat",
        "\"Montserrat\", system-ui, sans-serif",
    ),
    (
        "IBM Plex Sans",
        "ibm-plex-sans",
        "\"IBM Plex Sans\", system-ui, sans-serif",
    ),
    (
        "IBM Plex Sans Condensed",
        "ibm-plex-sans-condensed",
        "\"IBM Plex Sans Condensed\", \"Arial Narrow\", sans-serif",
    ),
    (
        "Source Serif 4",
        "source-serif-4",
        "\"Source Serif 4\", Georgia, serif",
    ),
    (
        "Space Grotesk",
        "space-grotesk",
        "\"Space Grotesk\", system-ui, sans-serif",
    ),
    ("Manrope", "manrope", "\"Manrope\", system-ui, sans-serif"),
    ("Archivo", "archivo", "\"Archivo\", system-ui, sans-serif"),
    (
        "Playfair Display",
        "playfair-display",
        "\"Playfair Display\", Georgia, serif",
    ),
    ("Jost", "jost", "\"Jost\", system-ui, sans-serif"),
    (
        "Instrument Sans",
        "instrument-sans",
        "\"Instrument Sans\", system-ui, sans-serif",
    ),
    (
        "Plus Jakarta Sans",
        "plus-jakarta-sans",
        "\"Plus Jakarta Sans\", system-ui, sans-serif",
    ),
    ("Outfit", "outfit", "\"Outfit\", system-ui, sans-serif"),
    (
        "Roboto Mono",
        "roboto-mono",
        "\"Roboto Mono\", ui-monospace, monospace",
    ),
    (
        "IBM Plex Mono",
        "ibm-plex-mono",
        "\"IBM Plex Mono\", ui-monospace, monospace",
    ),
];

pub const TEMPLATE_HTML_MAX: usize = 256 * 1024;
pub const TEMPLATE_CSS_MAX: usize = 256 * 1024;
pub const TEMPLATE_FIELDS_MAX: usize = 64;
pub const TEMPLATE_VARIABLES_MAX: usize = 32;
/// Variants (looks) per template: Classic + the 9 library styles, with room for 2 of the user's own.
pub const TEMPLATE_VARIANTS_MAX: usize = 12;
/// Longest variant name (characters, trimmed).
pub const VARIANT_NAME_MAX: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

/// A styling knob exposed as the CSS custom property `--<key>` on the layer host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum VariableKind {
    /// `#rrggbb` or `#rrggbbaa`.
    Color,
    /// One of `TEMPLATE_FONTS` by name.
    Font,
    /// A number in `min..=max`, emitted with `unit` (e.g. `px`, `%`, `` for unitless).
    Range {
        min: f64,
        max: f64,
        step: f64,
        unit: String,
    },
    /// One of `options[].value`.
    Select { options: Vec<SelectOption> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableDef {
    pub key: String,
    pub label: String,
    #[serde(flatten)]
    pub kind: VariableKind,
    /// The value as the user sees it (colour hex, font name, number as text, option value).
    pub default: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetRef {
    pub id: String,
    pub version: u32,
}

/// `template.json` (minus `schemaVersion`). Fields reuse `core::live::FieldDef`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateMeta {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub preset: Option<PresetRef>,
    #[serde(default)]
    pub fields: Vec<crate::core::live::FieldDef>,
    #[serde(default)]
    pub variables: Vec<VariableDef>,
    /// Take the graphic off air automatically this long after it goes in.
    #[serde(default)]
    pub auto_out_ms: Option<u32>,
    /// RFC 3339; set on every save.
    #[serde(default)]
    pub updated_at: String,
    /// Id of the variant layers show unless they pick another (one of the template's variants).
    #[serde(default)]
    pub default_variant: String,
    /// The design box: the size the template is drawn at for `fit` and where a new layer goes.
    /// `None` = none (omitted from the file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design: Option<TemplateDesign>,
}

/// How a layer's content fills its box (`fit` | `reflow`).
pub use crate::core::protocol::ContentSizing;

/// Sides of a template's design box, each.
pub const DESIGN_SIZE_RANGE: std::ops::RangeInclusive<u32> = 16..=8192;

/// A template's design box (`template.json` `design`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateDesign {
    /// Design width and height in px, [`DESIGN_SIZE_RANGE`].
    pub w: u32,
    pub h: u32,
    /// A new layer's default position on a 1920×1080 screen (scaled to the screen's size and
    /// clamped inside it); `None` = centered. Finite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// The sizing new layers get.
    #[serde(default)]
    pub sizing: ContentSizing,
}

/// A variant as `template.json` lists it: its own default values for the variables (missing
/// keys use the variable definition's default). Ids are slugs `[a-z][a-z0-9-]{0,31}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateVariant {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub vars: crate::core::protocol::Fields,
}

/// A variant with its look (`variants/<id>/template.html` + `template.css`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantDoc {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub vars: crate::core::protocol::Fields,
    pub html: String,
    pub css: String,
}

/// How an on-air layer changes variant (live edit `immediate`, automation).
pub use crate::core::protocol::VariantTransition;
pub use crate::core::protocol::{TakeTransition, TakeTransitionKind};

/// A whole template as the editor works with it (schemaVersion 2): the shared field schema and
/// variable definitions, plus every variant's look. `defaultVariant` is one of `variants`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateDoc {
    pub id: String,
    #[serde(flatten)]
    pub meta: TemplateMeta,
    /// In tab order; 1..=[`TEMPLATE_VARIANTS_MAX`].
    pub variants: Vec<VariantDoc>,
}

/// What lists show; read from `template.json` only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub preset: Option<PresetRef>,
    pub field_count: u32,
    pub variable_count: u32,
    /// Number of variants (1 for a single look).
    pub variant_count: u32,
    pub updated_at: String,
    /// The built-in preset it was made from has a newer look (`template_update::preset_update`).
    pub preset_update: Option<PresetUpdate>,
}

/// A preset template's look is outdated: made from preset version `from`, the app ships `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetUpdate {
    pub from: u32,
    pub to: u32,
}

/// What `update_template_from_preset` did: the template as saved and the `pre-update` backup
/// (`undo_template_update` restores the template from it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateUpdated {
    pub doc: TemplateDoc,
    pub backup: BackupInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: u32,
    /// The template type (`lowerThird`, `title`, `bug`, `ticker`, `nowNext`, `slideshow`,
    /// `chat`, `countdown`, `poll`, `scoreboard`) from `presets/library.json`, or none.
    pub kind: Option<String>,
    /// A legacy lower-third look (shown under "Legacy" in the gallery).
    pub legacy: bool,
    /// The variants it ships (style ids and names), default first.
    pub variants: Vec<PresetVariantInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetVariantInfo {
    pub id: String,
    pub name: String,
}

/// Where a new template comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TemplateSource {
    Blank,
    Preset { preset_id: String },
}

/// Unsaved editor state pushed to the preview screen. Everything is optional so the editor only
/// sends what changed; the server diffs and emits `layer` (HTML/CSS/field types), `vars` or
/// `fields` accordingly.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TemplateDraft {
    /// The variant being edited (its html/css/vars are what the preview shows).
    pub variant: Option<String>,
    pub html: Option<String>,
    pub css: Option<String>,
    pub fields: Option<Vec<crate::core::live::FieldDef>>,
    pub variables: Option<Vec<VariableDef>>,
    /// Preview-only field values (the editor's "Preview values").
    pub field_values: Option<crate::core::protocol::Fields>,
    /// Variable values as the user sees them (validated and converted to CSS server-side).
    pub variable_values: Option<crate::core::protocol::Fields>,
    /// Absent = unchanged, `null` = off, a number = on. Plain `Option<Option<_>>` would read
    /// `null` as "absent", so the editor couldn't switch auto-out off.
    #[serde(deserialize_with = "present")]
    pub auto_out_ms: Option<Option<u32>>,
}

// ---- Screens (screens/<id>.json) ------------------------------------------------------------

/// Width and height of a screen, each side.
pub const SCREEN_SIZE_RANGE: std::ops::RangeInclusive<u32> = 16..=8192;
pub const SCREEN_LAYERS_MAX: usize = 64;
pub const SCREEN_NAME_MAX: usize = 100;
/// Ruler guides per axis.
pub const SCREEN_GUIDES_MAX: usize = 64;
/// Named safe areas per screen.
pub const SCREEN_SAFE_AREAS_MAX: usize = 16;
pub const SAFE_AREA_NAME_MAX: usize = 40;

pub use crate::core::protocol::Rect;

/// Editor-only ruler guides of a screen, in screen px: `x` are vertical lines, `y` horizontal
/// ones. Integers within `-size..=2 * size`, at most [`SCREEN_GUIDES_MAX`] per axis, stored sorted
/// and unique. Never part of the live state or the renderer protocol.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScreenGuides {
    pub x: Vec<i32>,
    pub y: Vec<i32>,
}

impl ScreenGuides {
    pub fn is_empty(&self) -> bool {
        self.x.is_empty() && self.y.is_empty()
    }
}

/// An editor-only named safe area (an LED wall bezel, a cut-off) in screen px. Never part of the
/// live state or the renderer protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeArea {
    /// nanoid(10), unique within the screen.
    pub id: String,
    /// 1..=[`SAFE_AREA_NAME_MAX`] characters (trimmed).
    pub name: String,
    pub rect: Rect,
}

/// What a layer shows. Tagged by `kind`: a template, or a media file (video, audio, image).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ScreenLayerKind {
    Template {
        template_id: String,
        /// Field values by key. Keys the template no longer has are kept on disk but ignored.
        #[serde(default)]
        fields: crate::core::protocol::Fields,
        /// Variable overrides only, as the user sees them; missing keys use the template default.
        #[serde(default)]
        vars: crate::core::protocol::Fields,
        /// Field key → source column. A bound field takes its value from the source's current row
        /// (overriding `fields[key]`); a missing source/column/row falls back to `fields[key]`
        /// with a `sourceMissing` warning. Omitted from the file when empty.
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        bindings: std::collections::BTreeMap<String, SourceBinding>,
        /// The variant shown; absent = the template's default variant (stored only when it is
        /// not the default). A missing variant falls back to the default (`variantMissing`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
        /// Content sizing; absent = `reflow` (what layers did before sizing existed). Layers
        /// created from a template with a `design` get its `sizing` written explicitly. Content:
        /// on air it follows Studio mode / Live edit like `rect`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sizing: Option<ContentSizing>,
        /// Layer opacity 0–100 (absent = 100). Content: follows Live edit / Studio like `rect`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<u8>,
    },
    /// A media layer: plays one file of the media library. Content (like `rect`): changes follow
    /// Live edit / Studio mode. Playback (play / pause / stop / seek) is not content: it acts on
    /// Program at once (like timers) and is never saved.
    Media {
        media: MediaLayerConfig,
        /// Layer opacity 0–100 (absent = 100).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<u8>,
    },
}

impl ScreenLayerKind {
    /// The template layer's template id (`None` for media layers).
    pub fn template_id(&self) -> Option<&str> {
        match self {
            Self::Template { template_id, .. } => Some(template_id),
            Self::Media { .. } => None,
        }
    }

    /// The media layer's config (`None` for template layers).
    pub fn media(&self) -> Option<&MediaLayerConfig> {
        match self {
            Self::Media { media, .. } => Some(media),
            Self::Template { .. } => None,
        }
    }
}

pub use crate::core::protocol::{
    MediaEndAction, MediaFit, MediaKind, MediaPlayback, PlaybackState,
};

/// Transport on a media layer's Program playback (`media_control`; never content, never
/// saved). `seek` takes a position in ms (clamped to in..out).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaAction {
    Play,
    Pause,
    Stop,
    Restart,
    Seek,
}

/// Volume of a media layer (0–100).
pub const MEDIA_VOLUME_MAX: u8 = 100;

/// A media layer's file and how it plays (`ScreenLayerKind::Media`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaLayerConfig {
    /// `/assets/<name>` of a video, audio or image file in the project (a bare name is accepted
    /// on load and stored as `/assets/<name>`).
    pub asset: String,
    #[serde(default)]
    pub fit: MediaFit,
    /// Play range in ms; `outMs` null = the end of the file. 0 ≤ inMs < outMs ≤ duration.
    #[serde(default)]
    pub in_ms: u64,
    #[serde(default)]
    pub out_ms: Option<u64>,
    /// 0–100.
    #[serde(default = "default_volume")]
    pub volume: u8,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub end_action: MediaEndAction,
    /// Take plays from the in point (true) or shows the in frame paused (false).
    #[serde(default = "default_true")]
    pub autoplay: bool,
}

fn default_volume() -> u8 {
    MEDIA_VOLUME_MAX
}

fn default_true() -> bool {
    true
}

/// Preload status of a media layer across connected Program renderers (app previews excluded).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaReadiness {
    /// Renderers that decoded the first frame at the in point (or the whole image).
    pub ready: u32,
    /// Program renderers connected to the screen (browser sources + display outputs).
    pub total: u32,
    /// The first error a renderer reported, or null.
    pub error: Option<String>,
}

/// `LiveLayerView.media`: playback (Program), readiness and the file's duration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaLayerView {
    pub playback: MediaPlayback,
    pub readiness: MediaReadiness,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBinding {
    pub source_id: String,
    /// A column key of that source. For a `list` field the binding takes every row (columns
    /// matched by key) and `column` is `""`.
    pub column: String,
    /// `list` fields only: order the rows by a column before `limit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<ListSort>,
    /// `list` fields only: at most this many rows (after sorting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Which row a single-field binding reads: the source's current row (default), or for a
    /// schedule the NOW or NEXT item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<RowSelector>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RowSelector {
    #[default]
    Current,
    Now,
    Next,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSort {
    pub column: String,
    /// Compare as numbers (non-numbers last); otherwise case-insensitive text.
    #[serde(default)]
    pub numeric: bool,
    /// Largest / last first (a leaderboard: numeric + desc).
    #[serde(default)]
    pub desc: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenLayer {
    /// nanoid(10), unique within the screen.
    pub id: String,
    pub name: String,
    pub rect: Rect,
    #[serde(flatten)]
    pub kind: ScreenLayerKind,
    /// Position anchor; absent = free. While an axis is anchored the core recomputes that
    /// coordinate of `rect` (`core::anchors`). Omitted when absent (normalized: both axes null
    /// and `safe` off = absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<LayerAnchor>,
}

/// Horizontal anchor of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnchorH {
    Left,
    Center,
    Right,
}

/// Vertical anchor of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnchorV {
    Top,
    Middle,
    Bottom,
}

/// Per-axis position anchor of a layer (`h`/`v` null = that axis is free). `safe`: anchor inside
/// title safe (5 % margins on each side) instead of the screen edges. A hand move (position
/// changed, size unchanged) releases the moved axis; a resize keeps the anchor; a screen resize
/// re-places anchored layers (`core::anchors`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerAnchor {
    #[serde(default)]
    pub h: Option<AnchorH>,
    #[serde(default)]
    pub v: Option<AnchorV>,
    #[serde(default)]
    pub safe: bool,
}

/// `screens/<id>.json` (minus `schemaVersion`). `layers` order is the z-order, bottom first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenMeta {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// `"transparent"` or `#rrggbb` / `#rrggbbaa`.
    pub background: String,
    #[serde(default)]
    pub layers: Vec<ScreenLayer>,
    /// Editor-only ruler guides; omitted when there are none.
    #[serde(default, skip_serializing_if = "ScreenGuides::is_empty")]
    pub guides: ScreenGuides,
    /// Editor-only named safe areas; omitted when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safe_areas: Vec<SafeArea>,
    /// RFC 3339; set on every save.
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenDoc {
    pub id: String,
    #[serde(flatten)]
    pub meta: ScreenMeta,
}

/// What lists show (cached per project, like template summaries).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSummary {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub background: String,
    pub layer_count: u32,
    pub updated_at: String,
}

/// For `create_screen`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewScreen {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub background: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScreenWarningCode {
    /// The layer's template does not exist (deleted, or the file was edited by hand). The layer
    /// stays in the file but is not rendered.
    TemplateMissing,
    /// A field or variable value was dropped (unknown key or invalid value); defaults apply.
    InvalidValue,
    /// A bound field's source, column or current row doesn't exist; the layer's own value applies.
    SourceMissing,
    /// The layer's variant is not in its template (deleted or renamed by hand); the template's
    /// default variant is shown.
    VariantMissing,
    /// A media layer's file is not in `assets/` (deleted or renamed outside Kinetra). The layer
    /// stays in the file but is not rendered.
    AssetMissing,
}

/// A soft problem found while loading a screen; never stops it from loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenWarning {
    pub layer_id: String,
    pub code: ScreenWarningCode,
    pub message: String,
}

/// What the screen editor works with: the saved document, the live state of its layers (on air,
/// pending; layers with a missing template are absent), and soft warnings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenState {
    pub doc: ScreenDoc,
    pub live: crate::core::live::LiveScreenView,
    pub warnings: Vec<ScreenWarning>,
}

// ---- Sources (sources/<id>.json) ------------------------------------------------------------

pub const SOURCE_NAME_MAX: usize = 100;
pub const SOURCE_COLUMNS_MAX: usize = 16;
pub const SOURCE_ROWS_MAX: usize = 1000;
/// Bytes per cell.
pub const SOURCE_CELL_MAX: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceColumn {
    /// `[a-z][a-z0-9_-]{0,31}`, unique within the source.
    pub key: String,
    pub label: String,
    /// `image` cells hold an asset URL (`/assets/<file>`). Omitted when `text`.
    #[serde(default, skip_serializing_if = "ColumnKind::is_text")]
    pub kind: ColumnKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnKind {
    #[default]
    Text,
    Image,
}

impl ColumnKind {
    pub fn is_text(&self) -> bool {
        *self == ColumnKind::Text
    }
}

/// What a source holds. Tagged by `kind`: `text`, `schedule`, `api`, `poll`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SourceKind {
    /// A table: one row per item (e.g. one guest per row), stepped through with Next/Prev. A
    /// single value is a one-row table.
    Text {
        columns: Vec<SourceColumn>,
        /// Column key → cell text. Missing keys read as empty.
        #[serde(default)]
        rows: Vec<crate::core::protocol::Fields>,
        /// The current row; clamped to `rows` (meaningless when there are no rows).
        #[serde(default)]
        index: u32,
        /// Step automatically while a bound layer is on air. Absent = off.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auto: Option<AutoAdvance>,
    },
    /// A run of show: items with times; each item's status (NOW, UP NEXT, DELAYED, PREVIOUS,
    /// CANCELLED, upcoming) is computed server-side from server time and operator overrides.
    Schedule {
        #[serde(default)]
        items: Vec<ScheduleItem>,
        /// A delay also shifts every later item.
        #[serde(default)]
        ripple: bool,
        /// The operator's pick for UP NEXT (an item id); null = automatic (the first upcoming).
        #[serde(default)]
        next_override: Option<String>,
        /// Status labels shown on air (editable, e.g. for other languages).
        #[serde(default)]
        labels: ScheduleLabels,
    },
    /// An API source: fetched on an interval (`core::api_poll`), mapped into a table
    /// (`core::api_map`) with the same model as `text` (columns, rows, index, auto-advance).
    Api {
        request: ApiRequest,
        format: ApiFormat,
        mapping: ApiMapping,
        #[serde(default)]
        poll: ApiPoll,
        /// The mapped columns (from `mapping.columns`: key, label, kind).
        #[serde(default)]
        columns: Vec<SourceColumn>,
        /// The last good rows (cache, so outputs have data right after a restart).
        #[serde(default)]
        rows: Vec<crate::core::protocol::Fields>,
        /// The current row, clamped to `rows`.
        #[serde(default)]
        index: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auto: Option<AutoAdvance>,
        /// RFC 3339 of the fetch that produced `rows`, or null.
        #[serde(default)]
        fetched_at: Option<String>,
    },
    /// A live poll (`core::polls`): one row per answer, the poll's own values on every row.
    /// Linked to a server poll (`link`).
    Poll {
        #[serde(default)]
        link: Option<PollLink>,
        /// The last known server poll (so outputs have data after a restart and offline).
        #[serde(default)]
        cache: Option<PollData>,
        /// The server poll was deleted; `cache` stays on screen.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        gone: bool,
    },
}

// ---- API sources ------------------------------------------------------------------------

pub const API_POLL_MIN_MS: u32 = 1000;
pub const API_POLL_DEFAULT_MS: u32 = 10_000;
/// Longest poll interval (1 hour).
pub const API_POLL_MAX_MS: u32 = 3_600_000;
pub const API_RESPONSE_MAX_BYTES: usize = 2 * 1024 * 1024;
pub const API_URL_MAX: usize = 2048;
pub const API_HEADERS_MAX: usize = 32;
/// Header values (plain ones) and POST bodies.
pub const API_HEADER_VALUE_MAX: usize = 8 * 1024;
pub const API_BODY_MAX: usize = 64 * 1024;
/// Rows a test request returns.
pub const API_TEST_ROWS: usize = 50;
/// Longest test request body preview, in bytes.
pub const API_PREVIEW_MAX: usize = 64 * 1024;
/// Largest JSON response a test request returns parsed (for the tree picker).
pub const API_TEST_JSON_MAX: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApiFormat {
    #[default]
    Json,
    Csv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ApiMethod {
    #[default]
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "POST")]
    Post,
}

/// A request header. A secret header holds only `secret` (an id in the OS keychain), never the
/// value; set it with `set_source_secret`. `stored` (server-set, read-only, never written to the
/// file) says whether the keychain has a value for it on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiHeader {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiRequest {
    pub url: String,
    #[serde(default)]
    pub method: ApiMethod,
    #[serde(default)]
    pub headers: Vec<ApiHeader>,
    /// POST body (sent as-is; JSON content-type when it parses as JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiMappingColumn {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub kind: ColumnKind,
    /// JSON: a JSON Pointer within each row ("" = the row itself); CSV: the header name (or the
    /// 0-based column index as text when there is no header row).
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CsvDelimiter {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = ",")]
    Comma,
    #[serde(rename = ";")]
    Semicolon,
    #[serde(rename = "\t")]
    Tab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApiCsv {
    pub header: bool,
    pub delimiter: CsvDelimiter,
}

impl Default for ApiCsv {
    fn default() -> Self {
        Self {
            header: true,
            delimiter: CsvDelimiter::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiMapping {
    /// JSON: JSON Pointer to the rows array (or one object = one row); "" = the root. Unused for
    /// CSV.
    #[serde(default)]
    pub rows: String,
    #[serde(default)]
    pub columns: Vec<ApiMappingColumn>,
    #[serde(default)]
    pub csv: ApiCsv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApiPoll {
    /// `API_POLL_MIN_MS..=API_POLL_MAX_MS`; default `API_POLL_DEFAULT_MS`.
    pub interval_ms: u32,
    /// Poll only while a layer binds the source, a flow uses it or its editor is open.
    pub only_when_used: bool,
    /// Paused by the operator.
    pub paused: bool,
}

impl Default for ApiPoll {
    fn default() -> Self {
        Self {
            interval_ms: API_POLL_DEFAULT_MS,
            only_when_used: true,
            paused: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApiState {
    Ok,
    Fetching,
    /// Held (401/403, or a request that cannot be built) until Retry or a save.
    Error,
    Paused,
    /// Failing; retried after a growing wait (`nextAt`).
    Backoff,
    SecretMissing,
    /// Not polling: nothing uses the source.
    #[default]
    Idle,
}

/// Per API source; in `SourceSummary.api` and the `source-changed` event.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiStatus {
    pub state: ApiState,
    /// Server epoch ms.
    pub last_ok_at: Option<u64>,
    pub last_attempt_at: Option<u64>,
    pub next_at: Option<u64>,
    pub http_status: Option<u16>,
    /// Error text (never contains secret values).
    pub message: Option<String>,
    pub rows: u32,
}

// ---- Live polls --------------------------------------------------------------------------------
// The server's poll object and quota are in docs/API.md (Polls); `core::polls` runs the client.

pub const POLL_QUESTION_MAX: usize = 200;
pub const POLL_OPTION_MAX: usize = 80;
pub const POLL_OPTIONS_MIN: usize = 2;
pub const POLL_OPTIONS_MAX: usize = 8;
pub const POLL_DURATION_MIN_S: u32 = 30;
pub const POLL_DURATION_MAX_S: u32 = 86_400;

/// A poll's status on the server: `draft` (never opened), `open`, `closed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PollStatus {
    #[default]
    Draft,
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollOption {
    pub position: u32,
    pub label: String,
    pub votes: u64,
}

/// A poll as the app knows it: the server's poll object (docs/API.md).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PollData {
    pub id: String,
    /// The 6-character voting code (`K7P2QX`).
    pub code: String,
    /// `https://poll.kinetra.live/K7P2QX`.
    pub url: String,
    pub question: String,
    pub options: Vec<PollOption>,
    pub status: PollStatus,
    pub show_results: bool,
    pub duration_seconds: Option<u32>,
    pub total_votes: u64,
    pub max_votes: Option<u64>,
    pub vote_limit_reached: bool,
    pub round: u32,
    pub created_at: Option<String>,
    pub opened_at: Option<String>,
    pub closes_at: Option<String>,
    pub closed_at: Option<String>,
    pub delete_after: Option<String>,
    pub expires_at: Option<String>,
}

/// The poll a source shows, by its server id and code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollLink {
    pub id: String,
    pub code: String,
}

/// How a poll source's data is arriving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PollConn {
    /// Not in use (no stream): the source shows its last known results.
    #[default]
    Idle,
    Connecting,
    /// The live stream is up.
    Live,
    /// The stream dropped; trying again shortly.
    Reconnecting,
    /// The stream isn't available: fetching the totals every few seconds.
    Polling,
    /// No session: sign in to get live results.
    SignedOut,
    /// The poll was deleted on the server; the last results stay.
    Gone,
}

/// Per poll source; in `SourceSummary.poll` and `source-changed`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollSourceView {
    /// The linked server poll's id (null for unlinked sources).
    pub poll_id: Option<String>,
    pub code: Option<String>,
    pub conn: PollConn,
    /// `draft | open | closed | full` (null without data).
    pub state: Option<String>,
    pub total_votes: u64,
}

/// `poll-live` event: a poll's latest data and how it arrives (every poll with a stream or a
/// watcher). `poll` is null when only `conn` changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollLive {
    pub id: String,
    pub conn: PollConn,
    pub poll: Option<PollData>,
}

/// `GET /v1/polls/quota` (docs/API.md): read from the server's snake_case, sent to the UI as
/// camelCase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct PollQuota {
    /// `pro | business | enterprise | beta`, or null without `pro.polls`.
    pub plan: Option<String>,
    /// What to show (`business` is "Kinetra Team"); null without `pro.polls`.
    #[serde(default)]
    pub plan_name: Option<String>,
    pub limits: Option<PollLimits>,
    pub usage: PollUsage,
    pub month_resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct PollLimits {
    pub open_polls: u32,
    /// Null = unlimited.
    pub polls_per_month: Option<u32>,
    pub votes_per_poll: u64,
    pub options_per_poll: u32,
    pub max_duration_seconds: u32,
    pub lifetime_seconds: u64,
    pub delete_after_close_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(
    rename_all(serialize = "camelCase", deserialize = "snake_case"),
    default
)]
pub struct PollUsage {
    pub open_polls: u32,
    pub polls_this_month: u32,
    pub polls: u32,
}

/// `create_poll` / `update_poll`: what the editor sends. In `update_poll` a missing field is
/// left as it is; `durationSeconds: null` removes the time limit (`clearDuration`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PollInput {
    pub question: Option<String>,
    pub options: Option<Vec<String>>,
    pub duration_seconds: Option<u32>,
    /// `update_poll` only: remove the duration.
    pub clear_duration: bool,
    pub show_results: Option<bool>,
}

/// Result of `test_api_source`: one fetch with the draft, nothing saved.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTestResult {
    pub http_status: Option<u16>,
    /// Response headers with secret-looking values redacted.
    pub headers: Vec<(String, String)>,
    /// Pretty JSON or the first CSV lines, at most 64 KB.
    pub preview: String,
    /// Parsed JSON (for the tree picker), null for CSV or when too large.
    pub json: Option<serde_json::Value>,
    /// CSV header names (or "0".."n-1" without a header row), null for JSON.
    pub csv_columns: Option<Vec<String>>,
    /// The mapped rows (first `API_TEST_ROWS`).
    pub rows: Vec<crate::core::protocol::Fields>,
    /// Per mapping column: how many rows had a value.
    pub hits: std::collections::BTreeMap<String, u32>,
    pub error: Option<String>,
}

pub const SCHEDULE_ITEMS_MAX: usize = 500;

/// One schedule item. Times are local date-times `YYYY-MM-DDTHH:MM` (the app machine's time zone).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleItem {
    /// nanoid(10), unique within the schedule.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub location: String,
    pub start: String,
    pub end: String,
    /// Minutes late (0 = on time). With `ripple`, later items shift too.
    #[serde(default)]
    pub delay_min: u32,
    #[serde(default)]
    pub state: ItemState,
    /// Set by "Start now" (actual start, local date-time); overrides the planned start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Set by "End" / starting the next item (actual end).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
}

/// The operator's override of an item; `auto` = follow the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemState {
    #[default]
    Auto,
    Now,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScheduleLabels {
    pub now: String,
    pub next: String,
    pub delayed: String,
    pub previous: String,
    pub cancelled: String,
    pub upcoming: String,
}

impl Default for ScheduleLabels {
    fn default() -> Self {
        Self {
            now: "NOW".into(),
            next: "UP NEXT".into(),
            delayed: "DELAYED".into(),
            previous: "PREVIOUS".into(),
            cancelled: "CANCELLED".into(),
            upcoming: String::new(),
        }
    }
}

/// Computed status of a schedule item (also the `data-status` value on air).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemStatus {
    Previous,
    Now,
    Next,
    Delayed,
    Upcoming,
    Cancelled,
}

/// Columns a schedule exposes to bindings (lists and single fields). `countdown` is a timer
/// state counting down to the item's effective start (bind it to a `timer` field).
pub const SCHEDULE_COLUMNS: &[(&str, &str)] = &[
    ("title", "Title"),
    ("subtitle", "Subtitle"),
    ("location", "Location"),
    ("start", "Start"),
    ("end", "End"),
    ("day", "Day"),
    ("new_start", "New start"),
    ("label", "Status label"),
    ("status", "Status"),
    ("countdown", "Countdown"),
];

/// A schedule item as the operator sees it, with its computed status and effective times.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleItemView {
    pub id: String,
    pub title: String,
    pub location: String,
    pub status: ItemStatus,
    /// Effective start/end as local date-times (delays and actual starts/ends applied).
    pub start: String,
    pub end: String,
    /// Server epoch ms of the effective start/end.
    pub start_ms: u64,
    pub end_ms: u64,
    /// The planned start differs (delayed or moved).
    pub moved: bool,
}

/// Payload of `schedule-changed` and the reply of `schedule_action` / `schedule_view`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleView {
    pub source_id: String,
    pub items: Vec<ScheduleItemView>,
    pub now_id: Option<String>,
    pub next_id: Option<String>,
    /// Server epoch ms when the view was computed (for local countdowns).
    pub computed_at: u64,
    /// Whether "Undo" has something to undo.
    pub can_undo: bool,
}

/// An operator action on a schedule (Control). Each is saved and re-planned at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ScheduleAction {
    /// The item becomes NOW (actual start = now); a previous NOW item ends now.
    StartNow {
        item_id: String,
    },
    /// The item becomes PREVIOUS (actual end = now).
    End {
        item_id: String,
    },
    /// The item is UP NEXT; optionally it starts at `start` (local date-time).
    SetNext {
        item_id: String,
        start: Option<String>,
    },
    /// Adds minutes of delay (negative removes; never below 0).
    Delay {
        item_id: String,
        minutes: i32,
    },
    /// A new start (local date-time); the duration is kept.
    SetStart {
        item_id: String,
        start: String,
    },
    Cancel {
        item_id: String,
    },
    Restore {
        item_id: String,
    },
    /// Clears overrides of the item (state, delay, actual times).
    Auto {
        item_id: String,
    },
    /// Reverts the last action (one level).
    Undo,
}

/// Auto-advance (rotation) of a text source. Runs server-side, only while at least one layer
/// bound to the source is on air. Applies on air directly (automation bypasses `onTake`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoAdvance {
    /// Interval, `AUTO_ADVANCE_RANGE`.
    pub every_ms: u32,
    /// Wrap to the first row after the last; otherwise stop at the last row.
    #[serde(rename = "loop", default)]
    pub looped: bool,
    #[serde(default)]
    pub transition: AutoTransition,
    /// Slideshow: random order, reshuffled each cycle (no repeat across the seam).
    #[serde(default)]
    pub shuffle: bool,
    /// Slideshow: settings kept but not stepping (operator pause).
    #[serde(default)]
    pub paused: bool,
    /// Slideshow: a row's `duration` column (seconds) overrides `every_ms` for that row.
    #[serde(default)]
    pub item_durations: bool,
}

pub const AUTO_ADVANCE_RANGE: std::ops::RangeInclusive<u32> = 1_000..=3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoTransition {
    /// Bound fields change in place (no animation replay).
    #[default]
    Update,
    /// Bound on-air layers play out, the source steps, then they play in.
    Retake,
}

/// `sources/<id>.json` (minus `schemaVersion`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMeta {
    pub name: String,
    #[serde(flatten)]
    pub kind: SourceKind,
    /// RFC 3339; set on every save (not on a step).
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDoc {
    pub id: String,
    #[serde(flatten)]
    pub meta: SourceMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKindName {
    Text,
    Schedule,
    Api,
    Poll,
}

/// What lists show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub id: String,
    pub name: String,
    pub kind: SourceKindName,
    pub row_count: u32,
    pub index: u32,
    /// The current row (empty when there are no rows), for the Control strip.
    pub current: crate::core::protocol::Fields,
    pub columns: Vec<SourceColumn>,
    /// Ids of the screens with a layer bound to this source.
    pub used_by: Vec<String>,
    pub updated_at: String,
    /// Auto-advance settings (null = off).
    pub auto: Option<AutoAdvance>,
    /// Server epoch ms of the next automatic step while auto-advance is armed (a bound layer is
    /// on air); null otherwise. For Control's countdown.
    pub next_at: Option<u64>,
    /// Schedule sources: the NOW and UP NEXT item ids (null for text sources).
    pub now_id: Option<String>,
    pub next_id: Option<String>,
    /// API sources: fetch status; null for other kinds.
    #[serde(default)]
    pub api: Option<ApiStatus>,
    /// API sources: the response format (`csv` = what the UI calls a CSV link); null for other
    /// kinds.
    #[serde(default)]
    pub format: Option<ApiFormat>,
    /// Poll sources: test or linked, connection, state; null for other kinds.
    #[serde(default)]
    pub poll: Option<PollSourceView>,
}

/// For `step_source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SourceStep {
    /// Relative, clamped (no wrap): -1 = previous, 1 = next.
    By { delta: i32 },
    /// Absolute, clamped.
    Index { index: u32 },
}

// ---- Timers (Control) -------------------------------------------------------------------------

/// For `set_timer`: one state change of a `timer` field (never ticks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TimerAction {
    Start,
    Pause,
    /// Back to the field's default state (paused).
    Reset,
    /// Paused at this many ms (elapsed for up, remaining for down).
    Set {
        ms: u64,
    },
    Configure {
        dir: crate::core::protocol::TimerDir,
        end: crate::core::protocol::TimerEnd,
    },
}

// ---- Macros (<project>/macros.json, legacy) ---------------------------------------------------
//
// Read only to migrate them to Cue flows (`core::flows`); not part of the UI's types any more.

pub const MACROS_MAX: usize = 64;
pub const MACRO_STEPS_MAX: usize = 64;
pub const MACRO_WAIT_MAX_MS: u32 = 600_000;

/// One step of a legacy macro (migrated to a flow node by `flows::macro_step_node`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum MacroStep {
    Take {
        screen_id: String,
        layer_id: String,
    },
    Clear {
        screen_id: String,
        layer_id: String,
    },
    ClearScreen {
        screen_id: String,
    },
    StepSource {
        source_id: String,
        step: SourceStep,
    },
    SetFields {
        screen_id: String,
        layer_id: String,
        fields: crate::core::protocol::Fields,
    },
    Wait {
        ms: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Macro {
    /// nanoid(10).
    pub id: String,
    /// Trimmed, 1..=100 characters.
    pub name: String,
    pub steps: Vec<MacroStep>,
}

// ---- Assets (<project>/assets/) -------------------------------------------------------------

/// Image files that can be imported (lower-case extensions).
pub const ASSET_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "svg", "gif", "webp"];
/// Video files that can be imported (lower-case extensions).
pub const MEDIA_VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "webm", "mov"];
/// Audio files that can be imported (lower-case extensions).
pub const MEDIA_AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "m4a", "aac"];
/// Images: at most 20 MB.
pub const ASSET_IMAGE_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// Video and audio: at most 8 GiB.
pub const ASSET_MEDIA_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// The image limit (kept for the image-only paths: recipes' sample logos).
pub const ASSET_MAX_BYTES: u64 = ASSET_IMAGE_MAX_BYTES;

/// A file in the project's `assets/` folder (image, video or audio), served at
/// `/assets/<name>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetInfo {
    /// File name (path-safe, unique in `assets/`).
    pub name: String,
    /// The value image fields and image cells hold: `/assets/<name>` (URL-encoded name).
    pub url: String,
    pub bytes: u64,
    /// RFC 3339.
    pub modified_at: String,
    /// Screen and source ids that reference it.
    pub used_by: Vec<String>,
    /// Probed details (`core::media_probe`); absent when the file couldn't be probed yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaInfo>,
}

/// What the core read from a media file's headers (`core::media_probe`; no external tools).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub kind: MediaKind,
    /// mp4 (incl. m4a), webm, mov, mp3, wav, aac (ADTS), png, jpg, webp, gif, svg.
    pub container: String,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    /// "h264 High 4.1", "hevc", "vp8", "vp9", "av1", "prores", …
    pub video_codec: Option<String>,
    /// "aac", "mp3", "opus", "vorbis", "pcm", …
    pub audio_codec: Option<String>,
    /// Video with an alpha channel.
    pub alpha: bool,
    /// MP4/MOV: the index (moov) comes before the media data. None for other containers.
    pub fast_start: Option<bool>,
    pub issues: Vec<MediaIssue>,
}

/// Where a file plays (`MediaIssue.platforms`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaPlatform {
    Windows,
    Macos,
    Obs,
    Vmix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaIssueCode {
    Unsupported,
    NoAlpha,
    NeedsExtension,
    SlowStart,
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IssueSeverity {
    /// Won't play there.
    Error,
    /// Plays with a caveat.
    Warning,
}

/// A conversion that fixes an issue: a label and a copyable ffmpeg command for this file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaConvert {
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaIssue {
    pub code: MediaIssueCode,
    pub severity: IssueSeverity,
    pub platforms: Vec<MediaPlatform>,
    /// Operator copy, short ("Won't play in OBS: HEVC.").
    pub message: String,
    pub convert: Option<MediaConvert>,
}

/// `asset-import` events while importing (≤ 10/s per file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetImportProgress {
    /// Import job id (from `import_media`'s reply).
    pub job: String,
    /// The file's position in the job's `paths`: the stable key of a file (its name in `assets/`
    /// is sanitized and may get a `-2` suffix, so it can differ from the picked file's name).
    pub index: u32,
    /// The file's name in `assets/` (or the original file name when it failed before that).
    pub name: String,
    pub copied: u64,
    pub total: u64,
    pub done: bool,
    /// Set when this file failed (too large, unknown type, cancelled, disk error).
    pub error: Option<String>,
}

// ---- Recipes (presets/recipes/<id>/ → <project>/recipes/<id>.json) ---------------------------

/// Which Control panel a recipe gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecipePanel {
    Slideshow,
    Schedule,
    Countdown,
}

/// A built-in recipe in the gallery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub panel: RecipePanel,
    pub version: u32,
    /// Preset template ids it uses (for the gallery preview).
    pub templates: Vec<String>,
}

/// Where an installed recipe puts its layers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RecipeScreen {
    Existing {
        screen_id: String,
    },
    New {
        name: String,
        width: u32,
        height: u32,
    },
}

/// For `install_recipe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecipe {
    pub recipe_id: String,
    /// Name of the instance (and prefix of what it creates), 1..=100 characters.
    pub name: String,
    pub screen: RecipeScreen,
}

/// An installed recipe: what it created and its flows. `<project>/recipes/<id>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeInstance {
    /// nanoid(10).
    pub id: String,
    /// The built-in recipe it came from.
    pub recipe_id: String,
    pub name: String,
    pub panel: RecipePanel,
    /// The source it drives (slideshow: text; schedule/countdown: schedule).
    pub source_id: Option<String>,
    pub screen_id: String,
    pub layer_ids: Vec<String>,
    pub template_ids: Vec<String>,
    /// Legacy (schemaVersion 1): migrated to flows on open; always empty afterwards.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// The flows it created (schemaVersion 2).
    #[serde(default)]
    pub flow_ids: Vec<String>,
    /// RFC 3339.
    pub created_at: String,
}

pub const RULES_MAX: usize = 32;
pub const RULE_ACTIONS_MAX: usize = 16;

/// "When … then …". Evaluated only on the events its trigger names (no polling).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    /// nanoid(10), unique within the recipe.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    pub when: Trigger,
    pub then: Vec<RuleAction>,
    /// RFC 3339 of the last time it fired (maintained by the server).
    #[serde(default)]
    pub last_fired_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Trigger {
    /// A schedule item gets this status (any item, or `item_id`).
    ItemStatus {
        source_id: String,
        status: ItemStatus,
        #[serde(default)]
        item_id: Option<String>,
    },
    /// `minutes` before the UP NEXT item's effective start.
    BeforeNext { source_id: String, minutes: u32 },
    /// Local time of day `HH:MM`; `days` 0 = Monday … 6 = Sunday (empty = every day).
    TimeOfDay {
        at: String,
        #[serde(default)]
        days: Vec<u8>,
    },
    /// A countdown timer field reaches zero.
    TimerZero {
        screen_id: String,
        layer_id: String,
        key: String,
    },
    /// A layer goes on air (`on_air: true`) or off air.
    Layer {
        screen_id: String,
        layer_id: String,
        on_air: bool,
    },
    /// A slideshow (auto-advancing text source) completes a cycle.
    SlideshowWrap { source_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RuleAction {
    /// Take a layer; with `for_ms`, clear it again after that long.
    Take {
        screen_id: String,
        layer_id: String,
        #[serde(default)]
        for_ms: Option<u32>,
    },
    Clear {
        screen_id: String,
        layer_id: String,
    },
    RunMacro {
        macro_id: String,
    },
    StepSource {
        source_id: String,
        step: SourceStep,
    },
    /// Pause (`true`) or resume a slideshow / auto-advance.
    PauseSource {
        source_id: String,
        paused: bool,
    },
    SetFields {
        screen_id: String,
        layer_id: String,
        fields: crate::core::protocol::Fields,
    },
}

// ---- Flows (<project>/flows/<id>.json) --------------------------------------------------------
//
// One automation model (docs/ARCHITECTURE.md "Flows"). A flow is a graph of nodes joined by event wires
// (control flow) and data wires (values pulled when an event passes). `nodes`/`edges` are the
// deployed graph the engine runs; `draft` holds undeployed editor changes.
//
// Port ids (the `port` of an edge end). Event ports:
// - every node with an event input: `in`; triggers have no input and fire `fired`;
// - actions: `done`, and `failed` (fires instead of `done` on failure when wired);
// - logic: `out` unless listed: branch `then`/`else`; switch `case0`..`case15` + `default`;
//   gate inputs `in`/`open`/`close`; delay inputs `in`/`cancel`; waitUntil inputs
//   `in`/`release`, outputs `out`/`timeout`; once/counter inputs `in`/`reset`; sequence outputs
//   `s0`..`s7`; random outputs `r0`..`r7`; repeat inputs `in`/`stop`, outputs `body`/`done`.
// Data ports are named after their value: trigger outputs (`item`, `at`, `row`, `index`,
// `value`), data node outputs (a source column key, `row`, `rows`, `index`, `title`, …,
// `value`, `now`, `text`, `number`, `bool`, `onAir`, `pending`, `count`, `round`), inputs
// (`condition`, `value`, `ms`, `index`, `item`, `text`, `a`..`h` of format, `a`/`b` of math and
// compare, and for setFields one input per field key). `core::flow_nodes` is the port table.

pub const FLOWS_MAX: usize = 128;
pub const FLOW_NODES_MAX: usize = 200;
pub const FLOW_EDGES_MAX: usize = 400;
pub const FLOW_NAME_MAX: usize = 100;
/// Node runs per firing; past this the instance stops with "Too many steps".
pub const FLOW_STEPS_MAX: u32 = 256;
/// Nesting of `action.runFlow`.
pub const FLOW_RUN_DEPTH_MAX: u32 = 4;
/// Firings per second per flow (sliding window); extra firings are dropped and reported.
pub const FLOW_FIRINGS_PER_SEC: u32 = 10;
/// Running instances per project.
pub const FLOW_INSTANCES_MAX: usize = 64;
/// Delay / debounce / throttle / waitUntil timeout upper bound (24 h).
pub const FLOW_DELAY_MAX_MS: u64 = 86_400_000;
/// Switch cases.
pub const FLOW_CASES_MAX: usize = 16;
/// Sequence and random outputs, format inputs.
pub const FLOW_BRANCHES_MAX: u8 = 8;
/// Rounds of a counted Repeat.
pub const FLOW_REPEAT_MAX: u32 = 10_000;
/// A Repeat's next round never starts sooner than this after its previous one started.
pub const FLOW_REPEAT_PACE_MS: u64 = 100;

/// What a new trigger does while the flow's instance is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Retrigger {
    /// Stop the running instance and start again (the macro behaviour).
    #[default]
    Restart,
    /// Drop the new firing.
    Ignore,
}

/// A flow file. On disk with `schemaVersion: 1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    /// nanoid(10) (a migrated macro keeps its id).
    pub id: String,
    /// Trimmed, 1..=[`FLOW_NAME_MAX`] characters.
    pub name: String,
    /// Operator toggle: a disarmed flow never fires.
    #[serde(default)]
    pub armed: bool,
    #[serde(default)]
    pub retrigger: Retrigger,
    /// The recipe instance that created it, if any.
    #[serde(default)]
    pub origin: Option<String>,
    /// The deployed graph.
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    /// Undeployed editor changes; `None` when the editor matches what is deployed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<FlowDraft>,
    /// RFC 3339, maintained by the server.
    #[serde(default)]
    pub last_fired_at: Option<String>,
    /// RFC 3339.
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowDraft {
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    /// RFC 3339.
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct NodePos {
    pub x: f64,
    pub y: f64,
}

/// `{ id, position, type, config }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowNode {
    /// nanoid(10), unique within the flow.
    pub id: String,
    #[serde(default)]
    pub position: NodePos,
    #[serde(flatten)]
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireKind {
    Event,
    Data,
}

/// Types carried by data wires. `number`, `bool` and `time` convert to `text` implicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataType {
    Text,
    Number,
    Bool,
    /// An instant (server epoch ms); as text: local `HH:MM`.
    Time,
    /// Column key → text (a source row or a schedule item; schedule items carry `_id`).
    Row,
    /// A list value (rows), e.g. for list fields.
    Rows,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortRef {
    pub node: String,
    pub port: String,
}

/// A wire from an output port to an input port of the same kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowEdge {
    /// nanoid(10), unique within the flow.
    pub id: String,
    pub from: PortRef,
    pub to: PortRef,
    pub kind: WireKind,
}

/// Which row `data.sourceRow` reads (an `index` data input overrides it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "pick",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RowPick {
    Current,
    First,
    Last,
    Index { index: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NowNext {
    Now,
    Next,
}

/// A constant's typed value: `{ "kind": "number", "value": 3 }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Constant {
    Text(String),
    Number(f64),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MathOp {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
    /// Rounds `a` (b unused).
    Round,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    /// `a` is empty (b unused).
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseMode {
    Pause,
    Resume,
    Toggle,
}

/// Which schedule item `action.schedule` acts on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "pick",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ItemPick {
    /// The NOW item.
    Now,
    /// The UP NEXT item.
    Next,
    Item {
        item_id: String,
    },
    /// The `item` data input (a row with `_id`).
    Input,
}

/// A schedule operation (the operator actions of Control, minus Undo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ScheduleOp {
    StartNow,
    End,
    SetNext {
        #[serde(default)]
        start: Option<String>,
    },
    Delay {
        minutes: i32,
    },
    SetStart {
        start: String,
    },
    Cancel,
    Restore,
    Auto,
}

/// A node: `type` + `config`. The catalogue is `core/flow_nodes.rs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "config", rename_all_fields = "camelCase")]
pub enum NodeKind {
    // -- Triggers (no event input; fire `fired`) --
    /// A cue button in Control (hotkey `flow/<id>` in `UiState.control.hotkeys`). ≤ 1 per flow.
    #[serde(rename = "trigger.cue")]
    Cue {
        /// Button label; empty/absent = the flow name.
        #[serde(default)]
        label: Option<String>,
    },
    /// Out: `item` (row).
    #[serde(rename = "trigger.itemStatus")]
    ItemStatus {
        source_id: String,
        status: ItemStatus,
        #[serde(default)]
        item_id: Option<String>,
    },
    /// Out: `item` (row).
    #[serde(rename = "trigger.beforeNext")]
    BeforeNext { source_id: String, minutes: u32 },
    /// Local `HH:MM`; `days` 0 = Monday … 6 = Sunday (empty = every day). Out: `at` (time).
    #[serde(rename = "trigger.timeOfDay")]
    TimeOfDay {
        at: String,
        #[serde(default)]
        days: Vec<u8>,
    },
    #[serde(rename = "trigger.timerZero")]
    TimerZero {
        screen_id: String,
        layer_id: String,
        key: String,
    },
    /// Never fired by PANIC.
    #[serde(rename = "trigger.layer")]
    LayerAir {
        screen_id: String,
        layer_id: String,
        on_air: bool,
    },
    #[serde(rename = "trigger.slideshowWrap")]
    SlideshowWrap { source_id: String },
    /// The current row changed by any path. Out: `row` (row), `index` (number).
    #[serde(rename = "trigger.sourceRow")]
    SourceRowChanged { source_id: String },
    /// Fired by another flow's `action.runFlow`. Out: `value` (text).
    #[serde(rename = "trigger.runByFlow")]
    RunByFlow {},
    /// Fired by the Control API (`POST /api/flows/{id}/run`, the `run_flow` command): an
    /// outside operator action, so it runs while Automation is off (like a Cue); at most one per
    /// flow. Out: `value` (text, the request's `value` or empty).
    #[serde(rename = "trigger.rest")]
    Rest {},
    /// An API source's data changed: with `column`, that column of the current row
    /// (out `value`: text); without, any row (out `row`: the current row).
    #[serde(rename = "trigger.apiChanged")]
    ApiChanged {
        source_id: String,
        #[serde(default)]
        column: Option<String>,
    },
    /// OSC in (`core::osc`): a message whose address matches `address` (exact, `*` = one path
    /// segment). Out: `value` (text, the first argument as text).
    #[serde(rename = "trigger.osc")]
    Osc { address: String },
    /// MIDI in (`core::midi`): a note-on (velocity > 0) or a CC change from `device` (`None` =
    /// any) on `channel` (1–16, `None` = any). Out: `value` (number 0–127: the velocity or the
    /// CC value).
    #[serde(rename = "trigger.midi")]
    Midi {
        #[serde(default)]
        device: Option<String>,
        #[serde(default)]
        channel: Option<u8>,
        kind: MidiKind,
        number: u8,
    },
    /// A media layer reached its out point with end action `hold` or `clear` (never `loop`;
    /// never by pause, stop, clear or PANIC).
    #[serde(rename = "trigger.mediaEnded")]
    MediaEnded { screen_id: String, layer_id: String },

    // -- Data (no event ports; evaluated when pulled) --
    /// In: `index` (number, overrides `row`). Out: one text per column key, `row`, `rows`,
    /// `index`.
    #[serde(rename = "data.sourceRow")]
    SourceRow { source_id: String, row: RowPick },
    /// Out: `title`, `subtitle`, `location`, `start`, `end`, `status`, `label` (text),
    /// `exists` (bool), `item` (row).
    #[serde(rename = "data.scheduleItem")]
    ScheduleItem { source_id: String, which: NowNext },
    /// Out: `value` (text, the layer's current value).
    #[serde(rename = "data.fieldValue")]
    FieldValue {
        screen_id: String,
        layer_id: String,
        key: String,
    },
    /// Out: `onAir`, `pending` (bool).
    #[serde(rename = "data.layerState")]
    LayerState { screen_id: String, layer_id: String },
    /// Out: `value`.
    #[serde(rename = "data.constant")]
    Constant { value: Constant },
    /// Out: `now` (time).
    #[serde(rename = "data.timeNow")]
    TimeNow {},
    /// `{a}`…`{h}` placeholders (`{a:HH:mm}` formats a time). In: `a`..`h` (text). Out: `text`.
    #[serde(rename = "data.format")]
    Format { pattern: String },
    /// In: `a`, `b` (number; unwired = config). Out: `number`.
    #[serde(rename = "data.math")]
    Math {
        op: MathOp,
        #[serde(default)]
        a: f64,
        #[serde(default)]
        b: f64,
    },
    /// In: `a`, `b` (numbers compare as numbers, else as text; unwired = config). Out: `bool`.
    #[serde(rename = "data.compare")]
    Compare {
        op: CompareOp,
        #[serde(default)]
        a: String,
        #[serde(default)]
        b: String,
    },

    // -- Logic (event input `in` unless stated) --
    /// In: `condition` (bool). Out: `then`, `else`.
    #[serde(rename = "logic.branch")]
    Branch {},
    /// In: `value` (text). Out: `case0`.. per case, `default`.
    #[serde(rename = "logic.switch")]
    Switch { cases: Vec<String> },
    /// Inputs `in`, `open`, `close`. Out: `out` while open.
    #[serde(rename = "logic.gate")]
    Gate {
        #[serde(default = "yes")]
        open: bool,
    },
    /// Inputs `in`, `cancel`; data in `ms` (number, overrides). Out: `out`.
    #[serde(rename = "logic.delay")]
    Delay { ms: u64 },
    /// Until a local `HH:MM`, or the `release` input; optional timeout. Out: `out`, `timeout`.
    #[serde(rename = "logic.waitUntil")]
    WaitUntil {
        #[serde(default)]
        at: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// `out` after `ms` with no new event.
    #[serde(rename = "logic.debounce")]
    Debounce { ms: u64 },
    /// `out` at most once per `ms`; extra events are dropped.
    #[serde(rename = "logic.throttle")]
    Throttle { ms: u64 },
    /// `s0`..`s{count-1}` in order (2..=8).
    #[serde(rename = "logic.sequence")]
    Sequence { count: u8 },
    /// `out` the first time only, until `reset` (or deploy / project open).
    #[serde(rename = "logic.once")]
    Once {},
    /// Counts `in`; every `every` events `out`. Inputs `in`, `reset`. Data out `count`.
    #[serde(rename = "logic.counter")]
    Counter { every: u32 },
    /// One of `r0`..`r{count-1}` at random (2..=8).
    #[serde(rename = "logic.random")]
    Random {
        count: u8,
        #[serde(default)]
        no_repeat: bool,
    },
    /// A loop. Inputs `in` (start), `stop`. Out: `body` each round (the next round starts once
    /// the body's branch has finished, incl. its delays, plus `delay_ms`), `done` (after the
    /// last round or a `stop`); data out `round` (number, from 1). `count` 1..=10000, `None` =
    /// forever. Rounds start at most every [`FLOW_REPEAT_PACE_MS`]; the step limit is per round.
    #[serde(rename = "logic.repeat")]
    Repeat {
        #[serde(default)]
        count: Option<u32>,
        #[serde(default)]
        delay_ms: u64,
    },

    // -- Actions (inputs `in`; outputs `done`, `failed`; go on air directly) --
    /// With `for_ms`, a timed clear (only in the same on-air period).
    #[serde(rename = "action.take")]
    Take {
        screen_id: String,
        layer_id: String,
        #[serde(default)]
        for_ms: Option<u32>,
    },
    #[serde(rename = "action.clear")]
    Clear { screen_id: String, layer_id: String },
    #[serde(rename = "action.clearScreen")]
    ClearScreen { screen_id: String },
    /// Every screen, animated (like `live_clear_all`; never PANIC).
    #[serde(rename = "action.clearAll")]
    ClearAll {},
    /// One data input per field key (unwired = the config value); list fields take `rows`.
    #[serde(rename = "action.setFields")]
    SetFields {
        screen_id: String,
        layer_id: String,
        fields: crate::core::protocol::Fields,
    },
    /// In: `ms` (number, for `set`).
    #[serde(rename = "action.setTimer")]
    SetTimer {
        screen_id: String,
        layer_id: String,
        key: String,
        action: TimerAction,
    },
    /// In: `index` (number, overrides an `index` step).
    #[serde(rename = "action.stepSource")]
    StepSource { source_id: String, step: SourceStep },
    #[serde(rename = "action.pauseSource")]
    PauseSource { source_id: String, mode: PauseMode },
    /// In: `item` (row) when `item` is `input`. Recorded like an operator action (Undo).
    #[serde(rename = "action.schedule")]
    Schedule {
        source_id: String,
        item: ItemPick,
        op: ScheduleOp,
    },
    /// In: `value` (text) passed to the target's `trigger.runByFlow`.
    #[serde(rename = "action.runFlow")]
    RunFlow {
        flow_id: String,
        /// `done` fires when the target finishes (otherwise at once).
        #[serde(default)]
        wait: bool,
    },
    /// Shows another variant of a layer's template, on air at once (bypasses `onTake`) with
    /// `transition`.
    #[serde(rename = "action.setVariant")]
    SetVariant {
        screen_id: String,
        layer_id: String,
        variant: String,
        #[serde(default)]
        transition: VariantTransition,
    },
    /// Stops a flow's running instance, like the cue's Stop button (any flow, including this
    /// one: then this instance ends after the node and `done` doesn't fire). Not running =
    /// nothing to do, `done`.
    #[serde(rename = "action.stopFlow")]
    StopFlow { flow_id: String },
    /// Studio mode: takes the screen's Preview to Program (`transition` null = the project
    /// default). Fails when Studio mode is off.
    #[serde(rename = "action.takePreview")]
    TakePreview {
        screen_id: String,
        #[serde(default)]
        transition: Option<TakeTransition>,
    },
    /// In: `text` (overrides `message`). Debug log + `kinetra.log`; optional toast
    /// (≤ 1 per 2 s per flow).
    #[serde(rename = "action.log")]
    Log {
        #[serde(default)]
        message: String,
        #[serde(default)]
        toast: bool,
    },
    /// Media transport on a media layer's Program playback, at once (like Control's transport).
    /// In: `ms` (number, overrides `ms` for `seek`).
    #[serde(rename = "action.media")]
    Media {
        screen_id: String,
        layer_id: String,
        action: MediaAction,
        #[serde(default)]
        ms: Option<u64>,
    },
}

fn yes() -> bool {
    true
}

/// A soft problem found by validation or at run time (missing target), per node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowIssue {
    pub node: Option<String>,
    pub message: String,
}

/// A flow as the UI lists it: the file plus server-side state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowInfo {
    #[serde(flatten)]
    pub flow: Flow,
    /// Missing targets of the deployed graph.
    pub warnings: Vec<FlowIssue>,
    /// An instance is running.
    pub running: bool,
    /// The file is from a newer app version: shown, runs, never written.
    pub read_only: bool,
}

/// For `create_flow`: a new flow, deployed at once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewFlow {
    pub name: String,
    #[serde(default)]
    pub armed: bool,
    #[serde(default)]
    pub retrigger: Retrigger,
    /// An installed recipe instance id: the flow is that recipe's (its id is appended to the
    /// instance's `flowIds`). `None` = an own flow.
    #[serde(default)]
    pub origin: Option<String>,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

/// For `save_flow`: replaces a flow's deployed graph (validated, deployed at once; drops the
/// draft). Used by Control's list view edits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFlow {
    pub id: String,
    pub name: String,
    pub retrigger: Retrigger,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlowRun {
    Running,
    Finished,
    /// By Stop, PANIC, disarm, deploy, Automation off or a restart.
    Stopped,
    Failed,
}

/// Payload of `flow-state`: an instance started, finished, was stopped or failed (also a firing
/// dropped by a limit: `failed` with the message).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowState {
    pub id: String,
    pub state: FlowRun,
    /// The node running (Running), or that failed.
    pub node: Option<String>,
    /// Why it failed / stopped, or which actions were skipped.
    pub message: Option<String>,
    /// RFC 3339; for `running` also the flow's new `lastFiredAt`.
    pub at: String,
}

/// Payload of `flow-log`: a Log action asked for a toast (≤ 1 per 2 s per flow).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowLog {
    /// The flow.
    pub id: String,
    /// The Log node.
    pub node: String,
    pub message: String,
    /// RFC 3339.
    pub at: String,
}

/// A node that ran, in a `flow-debug` batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowDebugNode {
    pub node: String,
    /// RFC 3339 of its last run in this batch.
    pub at: String,
}

/// Payload of `flow-debug`: what ran in a subscribed flow since the last batch (≤ 10 batches per
/// second per flow; only while an editor is subscribed, see `subscribe_flow_debug`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowDebug {
    pub id: String,
    pub nodes: Vec<FlowDebugNode>,
    /// Event edges an event travelled along.
    pub edges: Vec<String>,
    /// Run-time errors per node (missing target, failed action, limit hit).
    pub errors: Vec<FlowIssue>,
}

/// Result of `check_flow`: validation without saving. `errors` block deploy; `warnings` don't.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowCheck {
    pub errors: Vec<FlowIssue>,
    pub warnings: Vec<FlowIssue>,
}

// ---- Flow templates (presets/flows/<id>.json, embedded) -------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlowTemplateCategory {
    Show,
    Schedule,
    Sponsors,
    Sports,
    Blocks,
}

/// What a template slot points at. Values in `AddFlowTemplate.slots`: a screen, source or flow
/// id; a layer as `screenId/layerId`; a timer or field as `screenId/layerId/key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlowSlotKind {
    Screen,
    Layer,
    TextSource,
    ScheduleSource,
    /// A timer field of a layer.
    Timer,
    /// Any field of a layer.
    Field,
    Flow,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowSlotPrefer {
    /// Prefer layers whose template came from this preset (e.g. "now-next").
    #[serde(default)]
    pub template: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowSlot {
    /// `[a-z][a-zA-Z0-9]{0,31}`, unique within the template.
    pub key: String,
    pub label: String,
    pub kind: FlowSlotKind,
    #[serde(default)]
    pub prefer: FlowSlotPrefer,
    #[serde(default)]
    pub optional: bool,
}

/// A built-in flow template in the gallery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowTemplateInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: FlowTemplateCategory,
    /// The first flow's main path as node labels, e.g. "Cue → Take → Delay → Clear layer".
    pub chain: String,
    pub slots: Vec<FlowSlot>,
    /// How many flows adding it creates (1 to 3).
    pub flows: u8,
}

/// For `add_flow_template`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddFlowTemplate {
    pub template_id: String,
    /// Name of the flow (default: the template name); multi-flow templates add " · <suffix>".
    #[serde(default)]
    pub name: Option<String>,
    /// Slot key → value (see [`FlowSlotKind`]). Every required slot must be present.
    pub slots: std::collections::BTreeMap<String, String>,
}

// ---- Project files watcher ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    Template,
    Screen,
    Source,
    /// An image in `assets/` (id = file name).
    Asset,
    /// `flows/<id>.json` (a valid change is deployed).
    Flow,
    /// `controls.json` (MIDI mappings; id = the file name).
    Controls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileChange {
    /// Created or modified outside Kinetra; reloaded and applied to the live state.
    Changed,
    Removed,
    /// Changed but failed to load; the last good state is kept. `message` says why.
    Invalid,
}

/// Payload of `project-files-changed`: a template or screen changed on disk outside Kinetra.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesChanged {
    pub kind: FileKind,
    pub id: String,
    pub change: FileChange,
    pub message: Option<String>,
}

/// For `Option<Option<T>>` fields with `#[serde(default)]`: a present key (even `null`) becomes
/// `Some(_)`; only an absent key stays `None`.
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_auto_out_distinguishes_absent_null_and_value() {
        let d = |json: &str| {
            serde_json::from_str::<TemplateDraft>(json)
                .unwrap()
                .auto_out_ms
        };
        assert_eq!(d("{}"), None, "absent = unchanged");
        assert_eq!(d(r#"{"autoOutMs":null}"#), Some(None), "null = off");
        assert_eq!(d(r#"{"autoOutMs":5000}"#), Some(Some(5000)));
    }

    #[test]
    fn flow_nodes_round_trip_as_type_and_config() {
        let json = r#"{"id":"n1","position":{"x":10.0,"y":20.0},"type":"action.take","config":{"screenId":"s","layerId":"l","forMs":5000}}"#;
        let n: FlowNode = serde_json::from_str(json).unwrap();
        assert_eq!(
            n.kind,
            NodeKind::Take {
                screen_id: "s".into(),
                layer_id: "l".into(),
                for_ms: Some(5000)
            }
        );
        assert_eq!(serde_json::to_string(&n).unwrap(), json);
        let empty = r#"{"id":"n2","position":{"x":0.0,"y":0.0},"type":"logic.once","config":{}}"#;
        let n: FlowNode = serde_json::from_str(empty).unwrap();
        assert_eq!(n.kind, NodeKind::Once {});
        assert_eq!(serde_json::to_string(&n).unwrap(), empty);
        let c: FlowNode = serde_json::from_str(
            r#"{"id":"n3","type":"data.constant","config":{"value":{"kind":"number","value":3}}}"#,
        )
        .unwrap();
        assert_eq!(
            c.kind,
            NodeKind::Constant {
                value: Constant::Number(3.0)
            }
        );
        assert!(
            serde_json::from_str::<FlowNode>(r#"{"id":"x","type":"nope","config":{}}"#).is_err()
        );
    }
}
