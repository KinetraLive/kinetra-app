// Mirrors src-tauri/src/core/model.rs. Change both together.

export interface Resolution {
  width: number;
  height: number;
}

export interface ProjectSettings {
  port: number;
  lanAccess: boolean;
  defaultResolution: Resolution;
  /** The project's style (a style id from `list_styles`), or null. Set by `apply_style` with the
   * project scope; new layers from library presets start on it. */
  style?: string | null;
  /** Overrides colours, fonts and the logo in every library template (see BrandKit). */
  brand?: BrandKit;
  /** OSC control (per project, shown in Preferences → Server). Absent = off. */
  osc?: OscSettings;
}

// ---- MIDI / OSC control ----

/**
 * OSC input on UDP `port` (1024–65535, default 9000). Messages from this computer are always
 * accepted; from other machines only when `ProjectSettings.lanAccess` is on AND the sender's IP
 * is in `allow` (IPv4/IPv6 literals, ≤ 32). Optional feedback: on-air status sent to
 * `feedback` (host:port) as `/kinetra/layer/<screenId>/<layerId>/onair <0|1>`.
 */
export interface OscSettings {
  enabled: boolean;
  port: number;
  allow: string[];
  feedback: { host: string; port: number } | null;
}
export const OSC_DEFAULT_PORT = 9000;

/** The OSC listener (event `osc-status`, command `osc_status`): off without a project or with
 * OSC disabled; `listening` on UDP `port` (`lan`: every interface, else this computer only). */
export type OscStatus =
  | { state: "off" }
  | { state: "listening"; port: number; lan: boolean }
  | { state: "error"; code: ServerErrorCode; port: number; message: string };

/** LED colour tables for MIDI feedback (on air = red, ready = green, off). */
export type MidiFeedbackProfile = "generic" | "apc40" | "apcMini" | "launchpad" | "xtouch";

/** Per machine (prefs.json `midi`), keyed by device (port) name. */
export interface MidiPrefs {
  /** Inputs that are listened to; a device not listed is on (new devices work at once). */
  inputs: Record<string, { enabled: boolean }>;
  /** LED feedback per device: the output port (by name; null = the output with the input's
   * name) and its profile. Absent = no feedback. */
  feedback: Record<string, { output: string | null; profile: MidiFeedbackProfile }>;
}

/** A MIDI device as the core sees it (event `midi-devices`, command `list_midi_devices`). */
export interface MidiDevice {
  name: string;
  input: boolean;
  output: boolean;
  /** Listening (input open). */
  active: boolean;
}

/** What a mapping listens to. `device` null = any device; `channel` null = any (1–16). */
export interface MidiSource {
  device: string | null;
  channel: number | null;
  kind: "note" | "cc";
  /** 0–127. */
  number: number;
}

/** What a mapping does. `command`: a registry command id the core understands:
 *  `live.toggle:<s>/<l>`, `live.take:<s>/<l>`, `live.clear:<s>/<l>`, `live.update:<s>/<l>`,
 *  `cue.go:<flowId>`, `source.next:<id>`, `source.prev:<id>`, `live.clearScreen:<s>`,
 *  `live.clearAll`, `live.panic`, `media.play:<s>/<l>`, `media.pause:<s>/<l>`,
 *  `media.stop:<s>/<l>` (notes: on note-on with velocity > 0; CC: when it crosses 64 upwards). `value`: a fader/knob (CC 0–127 → the target's range) on a layer: its opacity
 *  (0–100 %), a range variable (min..max, step) or a number field. */
export type ControlTarget =
  | { kind: "command"; commandId: string }
  | {
      kind: "value";
      screenId: string;
      layerId: string;
      target: { kind: "opacity" } | { kind: "var"; key: string } | { kind: "field"; key: string };
    };

export interface ControlMapping {
  /** nanoid(10). */
  id: string;
  source: MidiSource;
  target: ControlTarget;
  /** Send LED status back for command mappings on layers/cues (on air / ready). */
  feedback: boolean;
}

/** `controls.json` in the project (mappings follow the project). ≤ 512 mappings. A source used
 * by two mappings is a conflict: both are kept and shown; the core runs neither and reports it
 * (like keyboard conflicts). */
export interface ControlsDoc {
  mappings: ControlMapping[];
}

/** One captured MIDI message while learning (event `midi-learn`). */
export interface MidiLearned {
  device: string;
  channel: number;
  kind: "note" | "cc";
  number: number;
  value: number;
}

/**
 * Project brand kit. Variable precedence: layer override → brand kit → the variant's value →
 * the definition default. Colours/fonts validated like variable values (font = a TEMPLATE_FONTS
 * name). `logo` is an asset URL (`/assets/<name>`): the fallback for every `logo` image field
 * whose layer value is empty.
 */
export interface BrandKit {
  accent?: string | null;
  secondary?: string | null;
  text?: string | null;
  plate?: string | null;
  fontHeading?: string | null;
  fontBody?: string | null;
  logo?: string | null;
}

/** Shared variable keys of every library template (CSS `--<key>`). */
export const STYLE_VARIABLES = [
  "accent",
  "secondary",
  "text",
  "plate",
  "font-heading",
  "font-body",
  "radius",
  "speed",
] as const;

/** A built-in design style (`presets/styles.json`); its id is the variant id in every library
 * preset (e.g. `frame`). `classic` is the look the older presets had before the library. */
export interface StyleInfo {
  id: string;
  name: string;
  description: string;
}

export type ApplyStyleScope =
  | { kind: "project" }
  | { kind: "layer"; screenId: string; layerId: string };

/** Result of `apply_style`: what switched, and what couldn't (custom templates without the
 * style, e.g. not from a library preset). `undo` restores every layer's previous variant in one
 * step (null when nothing switched). */
export interface ApplyStyleResult {
  switched: number;
  /** Layers whose template has no such variant and no library preset to copy it from. */
  unchanged: number;
  /** Distinct templates among `unchanged` ("2 custom templates unchanged"). */
  unchangedTemplates: number;
  /** Variants copied into project templates from their preset. */
  added: number;
  undo: string | null;
}

export interface Project {
  /** Absolute path of the project directory. */
  path: string;
  name: string;
  /** RFC 3339. */
  createdAt: string;
  settings: ProjectSettings;
}

/** `entries` is capped at FOREIGN_ENTRY_CAP; show "1000+" at the cap. */
export type FolderKind =
  | { kind: "empty" }
  | { kind: "project"; name: string }
  | { kind: "nonEmptyForeign"; entries: number };

export const FOREIGN_ENTRY_CAP = 1000;

export interface RecentProject {
  path: string;
  name: string;
  /** RFC 3339. */
  lastOpenedAt: string;
  /** Always false from `listRecents`; filled in by the `recents-checked` event. */
  missing: boolean;
}

export type Theme = "system" | "light" | "dark";

export interface BackupPrefs {
  enabled: boolean;
  intervalMinutes: number;
  retention: number;
}

export const BACKUP_INTERVAL_RANGE = { min: 1, max: 240 } as const;
export const BACKUP_RETENTION_RANGE = { min: 1, max: 100 } as const;

export interface Prefs {
  theme: Theme;
  backup: BackupPrefs;
  /**
   * Machine-wide key bindings: command id → keys in canonical spelling ("Ctrl+Shift+P", "Shift+F3",
   * "Escape"); [] = unbound. Overrides a command's default keys (layer and cue keys are per project,
   * in `UiState.control.hotkeys`). Invalid ids/keys are dropped server-side; ≤ 500 entries of ≤ 4
   * keys. Absent in values from before it existed.
   */
  keybindings?: Record<string, string[]>;
  /** MIDI devices on this machine (inputs on/off, LED feedback). Absent = defaults. */
  midi?: MidiPrefs;
  /**
   * Simple hides Flows, the template code editor, MIDI/OSC and JSON API sources (CSV links stay);
   * hidden features keep working in projects. The server always returns it: a new install (no
   * prefs.json) starts "simple", an existing prefs.json without it loads as "advanced".
   */
  uiMode?: UiMode;
  /** WebView zoom of the app windows (main + Preferences), 0.5–2 in steps of 0.1; absent = 1. */
  zoom?: number;
  /** Display outputs on this machine (≤ 8). Absent = none. Validated and clamped server-side. */
  outputs?: OutputConfig[];
}

// ---- Display outputs (per machine, prefs.json `outputs`) ----

/** A connected display as the OS reports it (`list_displays`, `displays-changed`). All numbers
 * are physical pixels in the virtual desktop. */
export interface DisplayInfo {
  /** Stable id: the monitor's device interface path (EDID product + the connector instance).
   * Survives reboots and replugging into the same port. */
  id: string;
  /** Friendly name from the EDID ("DELL U2720Q"); falls back to the adapter's description. */
  name: string;
  /** EDID manufacturer + product + serial when known, for matching a display moved to another
   * port (`id` changes then). Empty when the EDID couldn't be read. */
  edidKey: string;
  x: number;
  y: number;
  width: number;
  height: number;
  /** Refresh rate in Hz (integer as Windows reports it, e.g. 60, 59, 144). */
  refreshHz: number;
  primary: boolean;
  /** The display Kinetra's main window is on (outputs there are allowed but warned about). */
  hasMainWindow: boolean;
}

export type OutputBackground =
  | { kind: "black" }
  | { kind: "green" }
  /** `#rrggbb`. */
  | { kind: "color"; color: string };

/** Green for "Fill on green" (chroma). */
export const CHROMA_GREEN = "#00b140";

/**
 * One display output (`kind` is the extension point: NDI / DeckLink later, not built now).
 * `mode: "fill"` shows the screen composited on `background`; `mode: "key"` shows its alpha as
 * white on black and pairs with the fill output named in `keyOf` (same screen, frame-synced when
 * the two displays allow one spanning window: adjacent, same size and refresh rate).
 */
export interface OutputConfig {
  /** nanoid(10). */
  id: string;
  kind: "display";
  /** 1–40 characters, unique ignoring case ("Output 1"). */
  name: string;
  /** Shown when on and its display is connected and a project is open. */
  enabled: boolean;
  display: {
    /** `DisplayInfo.id` it was assigned to. */
    id: string;
    /** Last seen name / EDID key / rect: for the "disconnected" warning and the fallback match. */
    name: string;
    edidKey: string;
    width: number;
    height: number;
  };
  /** A screen of the open project; ignored (key: inherited) for a key output. */
  screenId: string | null;
  mode: "fill" | "key";
  /** Fill outputs only; key outputs are always black. */
  background: OutputBackground;
  /** Key outputs: the fill output it keys for (must be a fill output). Null for fill outputs. */
  keyOf: string | null;
  alwaysOnTop: boolean;
  /** Media audio from this output (fill outputs; key outputs are always silent). Absent = on, to
   * the system default device. `device`: the audio output's label as the OS shows it
   * ("Speakers (Realtek(R) Audio)"); null = the system default. Labels, not ids: device ids differ
   * per web origin and port. */
  audio?: { enabled: boolean; device: string | null };
}

export const OUTPUTS_MAX = 8;

export type OutputState =
  /** Disabled. */
  | "off"
  /** On air: the window shows the screen. */
  | "showing"
  /** Enabled, but the display isn't connected (remembered; restored on replug). */
  | "displayMissing"
  /** Enabled, but no project is open (the window stays up, black). */
  | "noProject"
  /** Enabled, but the screen isn't in the open project (black). */
  | "screenMissing"
  /** The window couldn't be created / the display is used by another output. */
  | "failed";

/** Live status per output (`list_outputs` / `outputs-changed`, on change only). */
export interface OutputStatus {
  id: string;
  state: OutputState;
  /** The display it is on now (after a fallback match, may differ from the config's id). */
  displayId: string | null;
  /** Key outputs and their fill: true when both render in one spanning window (frame-locked). */
  frameLocked: boolean | null;
  /** A test pattern is showing instead of the screen. */
  pattern: TestPattern | null;
  /** Short operator-facing reason for failed / a note ("Moved to another port"). */
  message: string | null;
}

export type TestPattern = "bars" | "grid" | "safe" | "name";

export type UiMode = "simple" | "advanced";
export const ZOOM_MIN = 0.5;
export const ZOOM_MAX = 2;
export const ZOOM_STEP = 0.1;

/** Limits of `Prefs.keybindings` and `UiState.palette` (mirror model.rs). */
export const KEYBINDINGS_CAP = 500;
export const KEYS_PER_COMMAND = 4;
export const PALETTE_RECENT_CAP = 5;

export type ControlView = "grid" | "single";
export type LiveEdit = "immediate" | "onTake";

/** Control-mode layout. */
export interface ControlViewState {
  view: ControlView;
  gridColumns: number;
  selectedScreenId: string | null;
  hiddenScreenIds: string[];
  liveEdit: LiveEdit;
  /**
   * "<screenId>/<layerId>" → key combo: F1–F12, optionally "Shift+" (e.g. "Shift+F3"). Toggles the
   * layer while the Control page is focused. Invalid and duplicate combos are dropped server-side.
   */
  hotkeys: Record<string, string>;
  /**
   * Automation off (next to PANIC): no automatic trigger fires, auto-advance holds, a countdown
   * with `end: hide` stays on air. Cues and hotkeys still run. Persisted per project.
   */
  automationOff: boolean;
  /** Width of Control's right panel in px (dragged; 280..=640 server-side); absent = default. */
  panelWidth?: number | null;
  /** Studio mode (Preview/Program). Absent in older files = off. Changed only with `set_studio`
   * (`set_ui_state` keeps the stored value). */
  studio?: StudioSettings;
  /** Server-owned: the Studio Preview's visibility cues (`screenId → layerId → visible`), so a
   * restart keeps the Preview. Read-only for the UI (`set_ui_state` keeps the stored value). */
  studioPreview?: Record<string, Record<string, boolean>>;
}

export interface UiState {
  /** Sidebar pinned open; otherwise it is an icon rail that expands on hover. */
  sidebarPinned: boolean;
  control: ControlViewState;
  /** The screen layout editor's view options. Optional only for values from before it existed. */
  screenEditor?: ScreenEditorView;
  /** The command palette (per project). Optional only for values from before it existed. */
  palette?: PaletteState;
}

export interface PaletteState {
  /** The last commands run from the palette, newest first (≤ 5 valid, distinct ids server-side). */
  recent: string[];
}

/**
 * Aspect crop overlays of the layout editor (an unknown value is stored as "none"). The editor's
 * limits and defaults (grid size range, guide and safe area caps, DEFAULT_SCREEN_EDITOR) mirror
 * model.rs in src/lib/components/screens/editor/limits.ts, so routes that don't edit screens
 * don't carry them.
 */
export type CropOverlay = "none" | "9:16" | "4:5" | "1:1" | "4:3";

/** View options of the screen layout editor (editor-only; nothing reaches the output). */
export interface ScreenEditorView {
  /** Boxes snap while dragging (Alt disables it for one drag). */
  snap: boolean;
  /** Ruler guides shown (and snapped to). */
  guides: boolean;
  grid: boolean;
  /** Grid spacing in screen px, 2–512. */
  gridSize: number;
  /** Action safe: the central 93 % (SMPTE ST 2046-1). */
  actionSafe: boolean;
  /** Title safe: the central 90 % (SMPTE ST 2046-1). */
  titleSafe: boolean;
  centerCross: boolean;
  /** The screen's own named safe areas (`ScreenDoc.safeAreas`). */
  customSafe: boolean;
  /** An unknown value is stored as "none". */
  crop: CropOverlay;
}

export type BackupReason =
  | "open"
  | "timer"
  | "manual"
  | "pre-import"
  | "pre-move"
  | "pre-restore"
  | "pre-reset"
  | "pre-migrate"
  | "pre-update";

export interface BackupInfo {
  /** Directory name under `.backups/`. */
  id: string;
  createdAt: string;
  reason: BackupReason;
  files: number;
  bytes: number;
}

export type SettingsSection = "prefs" | "project" | "uiState" | "controls";

export interface FieldChange {
  /** Dotted path, e.g. `backup.intervalMinutes`. */
  key: string;
  from: unknown;
  to: unknown;
}

export interface SectionDiff {
  section: SettingsSection;
  /** Empty when present but identical to the current state. */
  changes: FieldChange[];
}

export interface ImportPreview {
  exportedAt: string;
  appVersion: string;
  sections: SectionDiff[];
}

export type MovePhase =
  | "scanning"
  | "snapshot"
  | "copying"
  | "verifying"
  | "switching"
  | "deleting"
  | "done";

export interface MoveProgress {
  phase: MovePhase;
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
}

export type ResetScope = "appearance" | "controlLayout" | "preferences" | "everything";

export interface AppInfo {
  version: string;
  tauriVersion: string;
  webviewVersion: string;
  configDir: string;
  logDir: string;
  /** Build channel, from `KINETRA_CHANNEL` at build time (default "stable"). Picks the update feed. */
  channel: BuildChannel;
  /** Short git commit of the build ("unknown" outside a git checkout). */
  commit: string;
}

export type BuildChannel = "stable" | "canary";

// ---- Native menu (set_app_menu) ----

/**
 * The app menu, built by the frontend from the command registry and sent with `set_app_menu`
 * whenever it changes (diffed: an identical spec is never sent). Rust updates items in place when
 * the structure (ids, kinds, order) is unchanged and rebuilds otherwise. Clicking an item emits
 * `menu` `{ id }` (the item's id = a registry command id) to the main window.
 * Windows: a menu bar on the main window only; `keys` is shown as text, never registered (the
 * registry's keydown listener runs shortcuts). macOS: `keys` become accelerators, and the app
 * menu (About, Preferences…, Hide, Quit) is added by Rust from the items with those ids.
 */
export interface MenuSpec {
  menus: MenuSubmenu[];
}
export type MenuNode = MenuItem | MenuSubmenu | MenuSeparator | MenuPredefined;
export interface MenuItem {
  kind: "item";
  /** Registry command id. Unique in the spec. */
  id: string;
  /** Translated title, with "&" mnemonics allowed ("&File"). */
  text: string;
  /** Display keys in canonical spelling ("Ctrl+Shift+P"), first binding only; null = none. */
  keys: string | null;
  enabled: boolean;
  /** A check or radio state; null/absent = a plain item. */
  checked?: boolean | null;
}
export interface MenuSubmenu {
  kind: "submenu";
  /** Stable id ("menu.file", "menu.file.recent"). */
  id: string;
  text: string;
  enabled: boolean;
  items: MenuNode[];
}
export interface MenuSeparator {
  kind: "separator";
}
/** OS-provided items: they act on the focused text field / window natively. Undo/Redo are not
 * predefined on Windows: use the registry's `edit.undo` / `edit.redo` items there. */
export interface MenuPredefined {
  kind: "predefined";
  item: "cut" | "copy" | "paste" | "selectAll" | "undo" | "redo";
}

// ---- Preferences window ----

/** Section ids of the Preferences window, in order (License is added later between About and
 * Advanced without moving anything). Old `/settings#<id>` anchors map through
 * `PREFERENCE_ALIASES` in components/settings/sections.ts. */
export type PreferenceSectionId =
  | "general"
  | "appearance"
  | "outputs"
  | "network"
  | "license"
  | "shortcuts"
  | "advanced"
  | "about";

// ---- Updates (tauri-plugin-updater; check is manual) ----

export interface UpdateCheck {
  /** A newer version exists on this build's channel. */
  available: boolean;
  version: string | null;
  /** Release notes (plain text) and date (RFC 3339), when the feed has them. */
  notes: string | null;
  date: string | null;
  /** Why Install is disabled right now (anything on air), or null. Rechecked by install_update. */
  blocked: string | null;
}

/** `update-progress` events while installing (≤ 10/s). */
export interface UpdateProgress {
  downloaded: number;
  total: number | null;
  phase: "download" | "install";
}

export type AppErrorCode =
  | "io"
  | "notFound"
  | "notAProject"
  | "notEmpty"
  | "newerVersion"
  | "invalid"
  | "noProject"
  | "cancelled"
  | "busy"
  | "updateCheck"
  | "onAir"
  | "proRequired"
  | "proLimited"
  | "offline"
  | "cloud"
  | "internal";

/** What every failed command rejects with. `cloud` errors carry the Kinetra API's own code
 * (`apiCode`, docs/API.md) and `details`. */
export interface AppError {
  code: AppErrorCode;
  message: string;
  apiCode?: string;
  details?: Record<string, unknown>;
}

// ---- Sign-in and licensing (mirrors core/cloud/service.rs; display only: the core decides) ----

export interface CloudActivation {
  id: string;
  machine_name: string | null;
  slot: string | null;
  created_at: string | null;
  last_refreshed_at: string | null;
  this_machine: boolean | null;
}

/** A license as `GET /v1/license` reports it (snake_case, as the API sends it). */
export interface CloudLicense {
  id: string;
  /** A stable code; show `plan_name`. */
  plan: string;
  /** What to show, e.g. "Kinetra Team" (absent from licenses cached by older versions). */
  plan_name?: string | null;
  kind: "free" | "subscription" | "perpetual" | string;
  status: "active" | "expired" | "revoked" | string;
  entitlements: string[];
  seats: number;
  seats_used: number;
  /** The plan includes a backup computer (absent from older servers). */
  backup_included?: boolean | null;
  backup_used: boolean;
  starts_at: string | null;
  ends_at: string | null;
  updates_until: string | null;
  revoked_at: string | null;
  beta_program_id: string | null;
  activations: CloudActivation[];
}

export interface UpgradeBlocked {
  license_id: string;
  plan: string;
  reason: string;
  activations: CloudActivation[];
}

export type SigninView =
  | { state: "idle" }
  | { state: "waiting"; userCode: string; url: string; expiresAt: number }
  | { state: "denied" }
  | { state: "expired" }
  | { state: "failed"; message: string };

export type LicenseNotice =
  | { kind: "clockWrong"; minutes: number }
  | { kind: "seatsFull"; code: "seats_full" | "backup_in_use" | "backup_not_included" | string; activations: CloudActivation[]; backup: boolean };

export interface BetaView {
  name: string;
  state: "upcoming" | "active" | "ended";
  endsAt: number;
  endMessage: string[];
  ended: boolean;
  /** A beta or canary build: the end screen blocks the app ("Download the stable version"). */
  blocking: boolean;
}

// ---- AI templates (mirrors core/cloud/ai.rs and api.rs; the core gates and checks) ----

export interface AiLimits {
  maxChars: number;
  maxColors: number;
  maxFonts: number;
  types: string[];
}

export interface AiTaskQuota {
  used: number;
  limit: number;
  period: "month" | "total" | string;
  resets_at?: number | null;
  day_used?: number | null;
  day_limit?: number | null;
  day_resets_at?: number | null;
}

/** `GET /v1/ai/quota` (snake_case, as the API sends it). */
export interface AiQuota {
  enabled: boolean;
  disabled_for_account: boolean;
  plan: string;
  plan_name?: string | null;
  generate_template: AiTaskQuota | null;
  tweak_template: AiTaskQuota | null;
}

export interface AiGenerateInput {
  prompt: string;
  templateType?: string | null;
  brandColors?: string[];
  fonts?: string[];
  width?: number | null;
  height?: number | null;
}

/** An AI job (`ai-job` events, `ai_jobs`). */
export interface AiJobView {
  id: string;
  task: "generate" | "tweak";
  status: "pending" | "succeeded" | "failed" | "canceled";
  /** The API's error code (`ai_declined`, …) or `import` when the result couldn't be added. */
  code: string | null;
  message: string | null;
  templateId: string | null;
  label: string;
}

export type LicenseStatus =
  | "none"
  | "active"
  | "reduced"
  | "offlineExpired"
  | "expired"
  | "notCovered"
  | "revoked"
  | "clockTampered";

/** The core's license state for display (`license_view`, event `license-changed`). Times are unix
 * seconds. `entitlements` is what the core gates on now; `pending` is what remains once the
 * show ends (a loss waiting for nothing to be on air). */
export interface LicenseView {
  signedIn: boolean;
  account: { email: string; name: string | null } | null;
  status: LicenseStatus;
  access: "full" | "reduced" | "free";
  plan: string | null;
  /** The plan's name to show, when the server sent it. */
  planName: string | null;
  entitlements: string[];
  /** The app's Pro features, unlocked or not (decided in the core). */
  features: { ai: boolean; polls: boolean; control: boolean };
  pending: string[] | null;
  endsAt: number | null;
  updatesUntil: number | null;
  fullUntil: number | null;
  reducedUntil: number | null;
  offlineDaysLeft: number;
  lastCheckAt: number | null;
  online: boolean | null;
  machineName: string;
  slot: string | null;
  licenses: CloudLicense[];
  upgradeBlocked: UpgradeBlocked | null;
  beta: BetaView | null;
  notice: LicenseNotice | null;
  welcome: boolean;
  /** Something is on air (screens like Beta Ended wait for the show to end). */
  onAir: boolean;
  signin: SigninView;
}

// ---- Output server (mirrors ServerStatus in model.rs) ---------------------------------------

export type ServerErrorCode = "portInUse" | "bindFailed";

/** The output server follows the open project: stopped when none is open. */
export type ServerStatus =
  | { state: "stopped" }
  | { state: "starting"; port: number }
  | {
      state: "running";
      port: number;
      /** Base URL on this machine, e.g. `http://127.0.0.1:7770`. */
      url: string;
      /** Base URL for other machines, when LAN access is on and an address was found. */
      lanUrl: string | null;
    }
  | { state: "error"; code: ServerErrorCode; port: number; message: string };

// ---- Live state (mirrors core/live.rs views and core/protocol.rs) ---------------------------

/** Screen and layer of the hardcoded output test. */
export const TEST_SCREEN_ID = "test";
export const TEST_LAYER_ID = "lower-third";

export type FieldType =
  | "text"
  | "multiline"
  | "image"
  | "video"
  | "color"
  | "number"
  | "html"
  /** Repeated rows: value = JSON array of objects (column key → text). */
  | "list"
  /** Time of day, rendered by the renderer; the value renderers get is the format (`HH:mm:ss`). */
  | "clock"
  /** Count up / down: value = `TimerState` as JSON; rendered by the renderer. */
  | "timer";

export interface FieldDef {
  key: string;
  label: string;
  type: FieldType;
  default: string;
  /** `list` fields: the row columns. */
  columns?: SourceColumn[];
  /** `clock` fields: `HH`, `mm`, `ss` tokens (default "HH:mm:ss"). */
  format?: string;
}

/** Most rows / bytes a `list` value may hold. */
export const LIST_ROWS_MAX = 100;
export const LIST_VALUE_MAX = 64 * 1024;

export type TimerDir = "up" | "down";
/** At zero, a countdown holds (`stop`) or takes its layer off air (`hide`). */
export type TimerEnd = "stop" | "hide";

/**
 * A `timer` field's value (JSON). Shown = `base` ± time since `at` while `run`; `base` while
 * paused. `at` is server epoch ms.
 */
export interface TimerState {
  run: boolean;
  at: number;
  base: number;
  dir: TimerDir;
  end: TimerEnd;
}

/** For `set_timer`: one state change (never ticks). */
export type TimerAction =
  | { action: "start" }
  | { action: "pause" }
  | { action: "reset" }
  | { action: "set"; ms: number }
  | { action: "configure"; dir: TimerDir; end: TimerEnd };

export interface ScreenInfo {
  id: string;
  name: string;
  width: number;
  height: number;
  /** "transparent" or a CSS color. */
  background: string;
}

export interface LiveLayerView {
  /** Media layers only: playback (Program), readiness and the file's duration. */
  media?: { playback: MediaPlayback; readiness: MediaReadiness; durationMs: number | null } | null;
  /** Template layers with a non-empty video or image field: preload status of those files.
   * Absent otherwise (media layers carry theirs in `media`). */
  readiness?: MediaReadiness | null;
  id: string;
  name: string;
  onAir: boolean;
  fields: Record<string, string>;
  fieldDefs: FieldDef[];
  /** On air with edits staged for the next take (liveEdit = onTake): "Changes on next take". */
  pending: boolean;
  /** The variant shown (with onTake staging: the staged one); "" when the template is missing. */
  variant?: string;
}

export interface LiveScreenView {
  screen: ScreenInfo;
  layers: LiveLayerView[];
}

// ---- Templates (mirrors the Templates block in model.rs) ------------------------------------

/** Reserved, unlisted screen the template editor previews into. */
export { PREVIEW_SCREEN_ID } from "./templates-common";
export const PREVIEW_LAYER_ID = "template";

/** Fonts templates may use, served by the output server at /fonts/<slug>.css. */
export const TEMPLATE_FONTS: ReadonlyArray<{ name: string; slug: string; stack: string }> = [
  { name: "Barlow", slug: "barlow", stack: '"Barlow", system-ui, sans-serif' },
  { name: "Barlow Condensed", slug: "barlow-condensed", stack: '"Barlow Condensed", "Arial Narrow", sans-serif' },
  { name: "Oswald", slug: "oswald", stack: '"Oswald", "Arial Narrow", sans-serif' },
  { name: "Montserrat", slug: "montserrat", stack: '"Montserrat", system-ui, sans-serif' },
  { name: "IBM Plex Sans", slug: "ibm-plex-sans", stack: '"IBM Plex Sans", system-ui, sans-serif' },
  { name: "IBM Plex Sans Condensed", slug: "ibm-plex-sans-condensed", stack: '"IBM Plex Sans Condensed", "Arial Narrow", sans-serif' },
  { name: "Source Serif 4", slug: "source-serif-4", stack: '"Source Serif 4", Georgia, serif' },
  { name: "Roboto Mono", slug: "roboto-mono", stack: '"Roboto Mono", ui-monospace, monospace' },
  { name: "IBM Plex Mono", slug: "ibm-plex-mono", stack: '"IBM Plex Mono", ui-monospace, monospace' },
  { name: "Space Grotesk", slug: "space-grotesk", stack: '"Space Grotesk", system-ui, sans-serif' },
  { name: "Manrope", slug: "manrope", stack: '"Manrope", system-ui, sans-serif' },
  { name: "Archivo", slug: "archivo", stack: '"Archivo", system-ui, sans-serif' },
  { name: "Playfair Display", slug: "playfair-display", stack: '"Playfair Display", Georgia, serif' },
  { name: "Jost", slug: "jost", stack: '"Jost", system-ui, sans-serif' },
  { name: "Instrument Sans", slug: "instrument-sans", stack: '"Instrument Sans", system-ui, sans-serif' },
  { name: "Plus Jakarta Sans", slug: "plus-jakarta-sans", stack: '"Plus Jakarta Sans", system-ui, sans-serif' },
  { name: "Outfit", slug: "outfit", stack: '"Outfit", system-ui, sans-serif' },
];

export const TEMPLATE_HTML_MAX = 256 * 1024;
export const TEMPLATE_CSS_MAX = 256 * 1024;
export const TEMPLATE_FIELDS_MAX = 64;
export const TEMPLATE_VARIABLES_MAX = 32;
/** Field and variable keys (also the CSS custom property name `--<key>`). */
export const TEMPLATE_KEY_PATTERN = /^[a-z][a-z0-9_-]{0,31}$/;

export interface SelectOption {
  value: string;
  label: string;
}

export type VariableKind =
  | { type: "color" }
  | { type: "font" }
  | { type: "range"; min: number; max: number; step: number; unit: string }
  | { type: "select"; options: SelectOption[] };

export type VariableDef = VariableKind & {
  key: string;
  label: string;
  /** As the user sees it: colour hex, font name, number as text, option value. */
  default: string;
};

export interface PresetRef {
  id: string;
  version: number;
}

// ---- Studio mode (Preview / Program) ----

export type TakeTransitionKind = "cut" | "crossfade" | "outIn";
export interface TakeTransition {
  kind: TakeTransitionKind;
  /** Crossfade duration (ms, 100..=5000); ignored by cut and outIn. */
  ms: number;
}

/** Per project (ui-state.json, control.studio). */
export interface StudioSettings {
  enabled: boolean;
  /** After a take: "keep" = Preview mirrors the new Program (empty diff); "clear" = every layer
   * cued out in Preview (a blank slate). */
  previewAfterTake: "clear" | "keep";
  transition: TakeTransition;
}

/**
 * One layer's entry in a screen's Preview diff (Preview = Program + diff). `visible`: cued in
 * (true) / out (false) / as Program (null). `staged`: staged content (fields, vars, variant,
 * template, rect) — the same staging "On take" uses outside Studio mode. `programChanged`:
 * automation changed this layer on Program since it was cued (shown until the next take or until
 * the cue is cleared).
 */
export interface LayerCue {
  layerId: string;
  visible: boolean | null;
  staged: boolean;
  programChanged: boolean;
}

/** What Control shows for a screen's Preview. Layers as they look in Preview. */
export interface PreviewView {
  screenId: string;
  /** Bumps on every change of the diff (for later "Update"/"Record cue"). */
  rev: number;
  /** Empty = Preview equals Program. */
  cues: LayerCue[];
  layers: LiveLayerView[];
  /** A T-bar transition is in progress (0..1), or null. */
  tbar: number | null;
}

/**
 * The one studio operation entry point (core `apply_studio_op`): every take/cue from Control,
 * hotkeys, the palette, Flows and later REST goes through it (layer locks will be enforced there).
 */
export type StudioOp =
  | { op: "cue"; layerId: string; visible: boolean | null }
  | { op: "take"; transition?: TakeTransition | null }
  | { op: "tbarBegin"; transition?: TakeTransition | null }
  | { op: "tbarSet"; p: number }
  | { op: "tbarEnd"; commit: boolean }
  | { op: "clear" };

/** Variant limits. Ids are slugs `[a-z][a-z0-9-]{0,31}`; the first variant of a migrated
 * template is `default`. */
export const TEMPLATE_VARIANTS_MAX = 12;
export const VARIANT_NAME_MAX = 40;

/** A variant as `template.json` lists it: its own default values for the variables (missing
 * keys use the variable definition's default). */
export interface TemplateVariant {
  id: string;
  name: string;
  vars: Record<string, string>;
}

/** A variant with its look (`variants/<id>/template.html` + `template.css`). */
export interface VariantDoc extends TemplateVariant {
  html: string;
  css: string;
}

/** How an on-air layer changes variant (live edit `immediate`, automation). */
export type VariantTransition = "crossfade" | "outIn";

/**
 * A whole template as the editor works with it (schemaVersion 2): the shared field schema and
 * variable definitions, plus every variant's look. `defaultVariant` is one of `variants`.
 */
/**
 * How a layer's content fills its box (`rect`):
 * - `fit`: the template renders at its design size (`TemplateDoc.design`) and scales uniformly
 *   to fit the box, centered (letterboxed, never stretched).
 * - `reflow`: the template fills the box and lays itself out (container units / queries).
 */
export type ContentSizing = "fit" | "reflow";

/** A template's design box: the size it is drawn at for `fit`, and where a new layer goes. */
export interface TemplateDesign {
  /** Design width and height in px, 16..=8192. */
  w: number;
  h: number;
  /** A new layer's default position on a 1920×1080 screen (scaled to the screen's size, clamped
   * inside it); absent = centered. */
  x?: number | null;
  y?: number | null;
  /** The sizing new layers get. */
  sizing: ContentSizing;
}

export interface TemplateDoc {
  id: string;
  name: string;
  description: string;
  preset: PresetRef | null;
  fields: FieldDef[];
  variables: VariableDef[];
  autoOutMs: number | null;
  /** The design box; null = none (fit then uses the screen's size). In template.json as
   * `design` (omitted when null). */
  design?: TemplateDesign | null;
  /** RFC 3339. */
  updatedAt: string;
  defaultVariant: string;
  /** In tab order; 1..=TEMPLATE_VARIANTS_MAX. */
  variants: VariantDoc[];
}

export interface TemplateSummary {
  id: string;
  name: string;
  description: string;
  preset: PresetRef | null;
  fieldCount: number;
  variableCount: number;
  /** Number of variants (1 for a single look). */
  variantCount: number;
  updatedAt: string;
  /** The built-in preset it was made from has a newer look. */
  presetUpdate: PresetUpdate | null;
}

/** A preset template made from preset version `from`; the app ships `to`. */
export interface PresetUpdate {
  from: number;
  to: number;
}

/** What `update_template_from_preset` did: the template as saved and its `pre-update` backup. */
export interface TemplateUpdated {
  doc: TemplateDoc;
  backup: BackupInfo;
}

export interface PresetInfo {
  id: string;
  name: string;
  description: string;
  version: number;
  /** The template type this preset is ("lowerThird", "title", "bug", "ticker", "nowNext",
   * "slideshow", "countdown", "scoreboard", "sponsorWall"), or null for one-off
   * presets. */
  kind?: string | null;
  /** Legacy lower-third looks (classic-bar, minimal-line, slide-in-accent, news-strap): shown
   * under "Legacy" in the gallery. */
  legacy?: boolean;
  /** Variants it ships (style ids + names), default first. */
  variants?: { id: string; name: string }[];
}

export type TemplateSource = { kind: "blank" } | { kind: "preset"; presetId: string };

/** Unsaved editor state for the preview screen; send only what changed. */
export interface TemplateDraft {
  /** The variant being edited (its html/css/vars are what the preview shows). */
  variant?: string;
  html?: string;
  css?: string;
  fields?: FieldDef[];
  variables?: VariableDef[];
  /** Preview-only field values. */
  fieldValues?: Record<string, string>;
  /** Variable values as the user sees them (validated server-side). */
  variableValues?: Record<string, string>;
  autoOutMs?: number | null;
}

// ---- Screens (mirrors the Screens block in model.rs) ----------------------------------------

/** Width and height of a screen, each side. */
export const SCREEN_SIZE_RANGE = { min: 16, max: 8192 } as const;
export const SCREEN_LAYERS_MAX = 64;
export const SCREEN_NAME_MAX = 100;

/** A layer's box in screen pixels; the template lays out inside it (nothing is scaled). */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** What a layer shows. Tagged by `kind`: a template, or a media file (video, audio, image). */
export type ScreenLayerKind = TemplateLayerKind | MediaLayerKind;

/**
 * A media layer: plays one file from the media library. Content (like `rect`): changes follow
 * Live edit / Studio mode. Playback (play / pause / stop / seek) is NOT content: it acts on
 * Program at once (like timers) and is never saved.
 */
export interface MediaLayerKind {
  kind: "media";
  media: MediaLayerConfig;
  /** Layer opacity 0–100 (absent = 100). */
  opacity?: number | null;
  anchor?: LayerAnchor | null;
}

export interface MediaLayerConfig {
  /** `/assets/<name>` of a video, audio or image file in the project. */
  asset: string;
  /** fill = cover (crop), fit = contain (letterbox, transparent), stretch = fill the box. */
  fit: MediaFit;
  /** Play range in ms; `outMs` null = the end of the file. 0 ≤ inMs < outMs ≤ duration. */
  inMs: number;
  outMs: number | null;
  /** 0–100. */
  volume: number;
  muted: boolean;
  /** At the out point: hold the last frame, take the layer off air, or loop to the in point. */
  endAction: MediaEndAction;
  /** Take plays from the in point (true, default) or shows the in frame paused (false). */
  autoplay: boolean;
}
export type MediaFit = "fill" | "fit" | "stretch";
export type MediaEndAction = "hold" | "clear" | "loop";

/**
 * Live playback of a media layer (Program). The position at server time `now` is
 * `posMs + (now - at)` while playing (then clamped / wrapped by out point and end action),
 * `posMs` otherwise. Every renderer derives it from the server clock (`hello.now`), so display
 * outputs, browser sources and previews play in step. `at` may be slightly in the future: a play
 * is scheduled ~120 ms ahead so every renderer starts on the same instant.
 */
export interface MediaPlayback {
  state: "playing" | "paused" | "stopped";
  /** Server time, ms since the epoch. */
  at: number;
  posMs: number;
}

/** Preload status of a layer's media across connected Program renderers (app previews excluded). */
export interface MediaReadiness {
  /** Renderers that decoded the first frame at the in point (or the whole image). */
  ready: number;
  /** Program renderers connected to the screen (browser sources + display outputs). */
  total: number;
  /** The first error a renderer reported ("Can't play this codec"), or null. */
  error: string | null;
}

export type TemplateLayerKind = {
  kind: "template";
  templateId: string;
  /** Field values by key. */
  fields: Record<string, string>;
  /** Variable overrides only, as the user sees them; missing keys use the template default. */
  vars: Record<string, string>;
  /**
   * Field key → source column. A bound field takes its value from the source's current row; a
   * missing source/column/row falls back to `fields[key]` (warning `sourceMissing`). Absent = none.
   */
  bindings?: Record<string, SourceBinding>;
  /** The variant shown; absent = the template's default variant (missing → default +
   * `variantMissing` warning). */
  variant?: string | null;
  /** Content sizing; absent = `reflow` (what layers did before sizing existed). New layers get
   * their template's `design.sizing` written explicitly. Content: on air it follows Studio mode /
   * Live edit like `rect`. */
  sizing?: ContentSizing | null;
  /** Layer opacity 0–100 (absent = 100). Content: follows Live edit / Studio like `rect`. */
  opacity?: number | null;
  /** Position anchor; absent = free (the rect stays where it is). While an axis is anchored the core
   * recomputes that coordinate of `rect` whenever the layer size or the screen size changes. */
  anchor?: LayerAnchor | null;
};

/**
 * Per-axis position anchor of a layer (`h`/`v` null = that axis is free). `safe`: anchor inside
 * title safe (5 % margins on each side, SMPTE ST 2046-1 90 %) instead of the screen edges.
 * Rules (core, one path): the 3×3 grid sets both axes; "Center horizontally / vertically" set one.
 * A hand move (rect position changes with the size unchanged) releases the axis it moved; a resize
 * keeps the anchor (the box grows from it). A screen resize re-places anchored layers.
 */
export interface LayerAnchor {
  h: "left" | "center" | "right" | null;
  v: "top" | "middle" | "bottom" | null;
  safe: boolean;
}

export type ScreenLayer = ScreenLayerKind & {
  /** nanoid(10), unique within the screen. */
  id: string;
  name: string;
  rect: Rect;
};
export type TemplateScreenLayer = ScreenLayer & TemplateLayerKind;
export type MediaScreenLayer = ScreenLayer & MediaLayerKind;

/** `screens/<id>.json`. `layers` order is the z-order, bottom first. */
export interface ScreenDoc {
  id: string;
  name: string;
  width: number;
  height: number;
  /** "transparent" or #rrggbb / #rrggbbaa. */
  background: string;
  layers: ScreenLayer[];
  /** Editor-only ruler guides; absent when there are none. Never reach the renderer. */
  guides?: ScreenGuides;
  /** Editor-only named safe areas; absent when there are none. Never reach the renderer. */
  safeAreas?: SafeArea[];
  /** RFC 3339. */
  updatedAt: string;
}

/**
 * Ruler guides in screen px: `x` are vertical lines, `y` horizontal ones. Integers within
 * -size..2×size, at most SCREEN_GUIDES_MAX (64, screens/editor/limits.ts) per axis; the backend
 * stores them sorted and unique.
 */
export interface ScreenGuides {
  x: number[];
  y: number[];
}

/** A named safe area (an LED wall bezel, a cut-off) in screen px. */
export interface SafeArea {
  /** nanoid(10), unique within the screen. */
  id: string;
  /** 1–SAFE_AREA_NAME_MAX (40) characters; at most SCREEN_SAFE_AREAS_MAX (16) per screen. */
  name: string;
  rect: Rect;
}

export interface ScreenSummary {
  id: string;
  name: string;
  width: number;
  height: number;
  background: string;
  layerCount: number;
  updatedAt: string;
}

export interface NewScreen {
  name: string;
  width: number;
  height: number;
  background: string;
}

export type ScreenWarningCode =
  | "templateMissing"
  | "invalidValue"
  | "sourceMissing"
  | "variantMissing"
  /** A media layer's file is not in assets/. */
  | "assetMissing";

/** A soft problem found while loading a screen; never stops it from loading. */
export interface ScreenWarning {
  layerId: string;
  code: ScreenWarningCode;
  message: string;
}

/**
 * What the screen editor works with: the saved document, the live state of its layers (on air,
 * pending; layers with a missing template are absent) and soft warnings.
 */
export interface ScreenState {
  doc: ScreenDoc;
  live: LiveScreenView;
  warnings: ScreenWarning[];
}

// ---- Project files watcher ------------------------------------------------------------------

export type FileKind = "template" | "screen" | "source" | "asset" | "flow" | "controls";
/** `invalid`: changed but failed to load; the last good state is kept (`message` says why). */
export type FileChange = "changed" | "removed" | "invalid";

/** Payload of `project-files-changed`: a template or screen changed on disk outside Kinetra. */
export interface FilesChanged {
  kind: FileKind;
  id: string;
  change: FileChange;
  message: string | null;
}

// ---- Sources (mirrors the Sources block in model.rs) ----------------------------------------

export const SOURCE_NAME_MAX = 100;
export const SOURCE_COLUMNS_MAX = 16;
export const SOURCE_ROWS_MAX = 1000;
/** Bytes per cell. */
export const SOURCE_CELL_MAX = 4 * 1024;

export interface SourceBinding {
  sourceId: string;
  /** A column key of that source; `""` for a `list` field (it takes every row). */
  column: string;
  /** `list` fields only: order rows by a column before `limit`. */
  sort?: ListSort;
  /** `list` fields only: at most this many rows. */
  limit?: number;
  /** Single fields: the source's current row (default), or a schedule's NOW / NEXT item. */
  row?: RowSelector;
}

export type RowSelector = "current" | "now" | "next";

export interface ListSort {
  column: string;
  /** Compare as numbers (non-numbers last); otherwise case-insensitive text. */
  numeric: boolean;
  /** Largest / last first (a leaderboard: numeric + desc). */
  desc: boolean;
}

export interface SourceColumn {
  /** [a-z][a-z0-9_-]{0,31}, unique within the source. */
  key: string;
  label: string;
  /** `image` cells hold an asset URL (`/assets/<file>`); absent = text. */
  kind?: ColumnKind;
}

export type ColumnKind = "text" | "image";

/** What a source holds; tagged by `kind`. */
export type SourceKind = {
  kind: "text";
  columns: SourceColumn[];
  /** Column key → cell text; missing keys read as empty. */
  rows: Record<string, string>[];
  /** The current row, clamped to `rows`. */
  index: number;
  /** Step automatically while a bound layer is on air; absent = off. */
  auto?: AutoAdvance;
} | {
  /** An API source: fetched on an interval, mapped into a table (same model as "text"). */
  kind: "api";
  request: ApiRequest;
  format: ApiFormat;
  mapping: ApiMapping;
  poll: ApiPoll;
  /** The mapped columns (from `mapping.columns`: key, label, kind). */
  columns: SourceColumn[];
  /** The last good rows (cache, so outputs have data right after a restart). */
  rows: Record<string, string>[];
  /** The current row, clamped to `rows`. */
  index: number;
  auto?: AutoAdvance;
  /** RFC 3339 of the last good fetch, or null. */
  fetchedAt: string | null;
};

// ---- Live polls (docs/API.md "Polls"; core/polls.rs) ----

/**
 * `sources/<id>.json` of kind `poll` (SourceKind::Poll in model.rs; like schedules, not a
 * table): one row per answer, the poll's own values on every row. Linked to a server poll
 * (`link`).
 */
export interface PollSourceDoc {
  id: string;
  name: string;
  kind: "poll";
  link: PollLink | null;
  /** The last known server poll (outputs keep it after a restart and offline). */
  cache: PollData | null;
  /** The server poll was deleted; `cache` stays on screen. Omitted when false. */
  gone?: boolean;
  updatedAt: string;
}

export const POLL_QUESTION_MAX = 200;
export const POLL_OPTION_MAX = 80;
export const POLL_OPTIONS_MIN = 2;
export const POLL_OPTIONS_MAX = 8;
export const POLL_DURATION_MIN_S = 30;
export const POLL_DURATION_MAX_S = 86_400;

/** A poll's status on the server: draft (never opened), open, closed. */
export type PollStatus = "draft" | "open" | "closed";

export interface PollOption {
  position: number;
  label: string;
  votes: number;
}

/** A poll as the app knows it: the server's poll object. */
export interface PollData {
  id: string;
  /** The 6-character voting code ("K7P2QX"). */
  code: string;
  /** "https://poll.kinetra.live/K7P2QX". */
  url: string;
  question: string;
  options: PollOption[];
  status: PollStatus;
  showResults: boolean;
  durationSeconds: number | null;
  totalVotes: number;
  maxVotes: number | null;
  voteLimitReached: boolean;
  round: number;
  createdAt: string | null;
  openedAt: string | null;
  closesAt: string | null;
  closedAt: string | null;
  deleteAfter: string | null;
  expiresAt: string | null;
}

export interface PollLink {
  id: string;
  code: string;
}

/** How a poll source's data arrives. */
export type PollConn = "idle" | "connecting" | "live" | "reconnecting" | "polling" | "signedOut" | "gone";

/** Per poll source, in `SourceSummary.poll`. */
export interface PollSourceView {
  pollId: string | null;
  code: string | null;
  conn: PollConn;
  /** "draft" | "open" | "closed" | "full", or null without data. */
  state: string | null;
  totalVotes: number;
}

/** `poll-live`: a poll's latest data (`poll`) or a change of how it arrives (`poll` null). */
export interface PollLive {
  id: string;
  conn: PollConn;
  poll: PollData | null;
}

/** GET /v1/polls/quota. `plan` and `limits` are null without `pro.polls`. */
export interface PollQuota {
  plan: string | null;
  /** What to show (`business` is "Kinetra Team"); null without `pro.polls`. */
  planName: string | null;
  limits: PollLimits | null;
  usage: PollUsage;
  monthResetsAt: string | null;
}

export interface PollLimits {
  openPolls: number;
  /** null = unlimited. */
  pollsPerMonth: number | null;
  votesPerPoll: number;
  optionsPerPoll: number;
  maxDurationSeconds: number;
  lifetimeSeconds: number;
  deleteAfterCloseSeconds: number;
}

export interface PollUsage {
  openPolls: number;
  pollsThisMonth: number;
  polls: number;
}

/** createPoll / updatePoll. In an update a missing field is left as it is. */
export interface PollInput {
  question?: string;
  options?: string[];
  durationSeconds?: number;
  /** updatePoll only: remove the time limit. */
  clearDuration?: boolean;
  showResults?: boolean;
}

// ---- API sources ----

export const API_POLL_MIN_MS = 1000;
/** One hour (the backend refuses longer intervals). */
export const API_POLL_MAX_MS = 3_600_000;
export const API_POLL_DEFAULT_MS = 10000;
export const API_RESPONSE_MAX_BYTES = 2 * 1024 * 1024;

export type ApiFormat = "json" | "csv";
export type ApiMethod = "GET" | "POST";

/** A request header. A secret header holds only `secret` (an id in the OS keychain), never the
 * value; set it with `setSourceSecret`. `stored` (server-set, read-only) says whether the
 * keychain has a value for it on this machine. */
export interface ApiHeader {
  name: string;
  value?: string | null;
  secret?: string | null;
  stored?: boolean;
}

export interface ApiRequest {
  url: string;
  method: ApiMethod;
  headers: ApiHeader[];
  /** POST body (sent as-is; JSON content-type when it parses as JSON). */
  body?: string | null;
}

export interface ApiMappingColumn {
  key: string;
  label: string;
  kind: "text" | "image";
  /** JSON: a JSON Pointer within each row ("" = the row itself); CSV: the header name (or the
   * 0-based column index as text when there is no header row). */
  path: string;
}

export interface ApiMapping {
  /** JSON: JSON Pointer to the rows array (or one object = one row); "" = the root. Unused for CSV. */
  rows: string;
  columns: ApiMappingColumn[];
  csv: { header: boolean; delimiter: "auto" | "," | ";" | "\t" };
}

export interface ApiPoll {
  /** >= API_POLL_MIN_MS; default API_POLL_DEFAULT_MS. */
  intervalMs: number;
  /** Poll only while a layer binds the source, a flow uses it or its editor is open. */
  onlyWhenUsed: boolean;
  /** Paused by the operator. */
  paused: boolean;
}

export type ApiState = "ok" | "fetching" | "error" | "paused" | "backoff" | "secretMissing" | "idle";

/** Per API source; in SourceSummary.api and the source-changed event. */
export interface ApiStatus {
  state: ApiState;
  /** Server epoch ms. */
  lastOkAt: number | null;
  lastAttemptAt: number | null;
  nextAt: number | null;
  httpStatus: number | null;
  /** Error text (never contains secret values). */
  message: string | null;
  rows: number;
}

/** Result of `testApiSource`: one fetch with the draft, nothing saved. */
export interface ApiTestResult {
  httpStatus: number | null;
  /** Response headers with secret-looking values redacted. */
  headers: [string, string][];
  /** Pretty JSON or the first CSV lines, at most 64 KB. */
  preview: string;
  /** Parsed JSON (for the tree picker), null for CSV or when too large. */
  json: unknown | null;
  /** CSV header names (or column count as "0".."n-1"), null for JSON. */
  csvColumns: string[] | null;
  /** The mapped rows (first 50). */
  rows: Record<string, string>[];
  /** Per mapping column: how many rows had a value. */
  hits: Record<string, number>;
  error: string | null;
}

/** `sources/<id>.json`. */
export type SourceDoc = SourceKind & {
  id: string;
  name: string;
  /** RFC 3339; set on every save (not on a step). */
  updatedAt: string;
};

export type SourceKindName = "text" | "schedule" | "api" | "poll";

export interface SourceSummary {
  id: string;
  name: string;
  kind: SourceKindName;
  rowCount: number;
  index: number;
  /** The current row (empty when there are no rows). */
  current: Record<string, string>;
  columns: SourceColumn[];
  /** Ids of the screens with a layer bound to this source. */
  usedBy: string[];
  updatedAt: string;
  auto: AutoAdvance | null;
  /** Server epoch ms of the next automatic step while armed, else null. */
  nextAt: number | null;
  /** Schedule sources: the NOW and UP NEXT item ids (null for text sources). */
  nowId: string | null;
  nextId: string | null;
  /** API sources: fetch status; null for other kinds. */
  api?: ApiStatus | null;
  /** API sources: the response format ("csv" = a CSV link); null for other kinds. */
  format?: ApiFormat | null;
  /** Poll sources: test or linked, connection, state; null for other kinds. */
  poll?: PollSourceView | null;
}

/**
 * Auto-advance (rotation): server-side, only while a bound layer is on air; applies on air
 * directly (bypasses "on take").
 */
export interface AutoAdvance {
  /** AUTO_ADVANCE_RANGE. */
  everyMs: number;
  /** Wrap after the last row; otherwise stop there. */
  loop: boolean;
  /** `update`: fields change in place; `retake`: bound layers play out, step, play in. */
  transition: "update" | "retake";
  /** Slideshow: random order, reshuffled each cycle. */
  shuffle?: boolean;
  /** Slideshow: kept but not stepping (operator pause). */
  paused?: boolean;
  /** Slideshow: a row's `duration` column (seconds) overrides `everyMs`. */
  itemDurations?: boolean;
}

export const AUTO_ADVANCE_RANGE = { min: 1_000, max: 3_600_000 } as const;

/** For `step_source`: relative (clamped, no wrap) or absolute. */
export type SourceStep = { to: "by"; delta: number } | { to: "index"; index: number };

// ---- Schedules (SourceKind::Schedule in model.rs) --------------------------------------------

export const SCHEDULE_ITEMS_MAX = 500;

/** The operator's override of an item; `auto` = follow the clock. */
export type ItemState = "auto" | "now" | "done" | "cancelled";

/** One schedule item. Times are local date-times `YYYY-MM-DDTHH:MM`. */
export interface ScheduleItem {
  id: string;
  title: string;
  subtitle: string;
  location: string;
  start: string;
  end: string;
  /** Minutes late (0 = on time). */
  delayMin: number;
  state: ItemState;
  /** Actual start/end set by Start now / End. */
  startedAt?: string;
  endedAt?: string;
}

export interface ScheduleLabels {
  now: string;
  next: string;
  delayed: string;
  previous: string;
  cancelled: string;
  upcoming: string;
}

/** `sources/<id>.json` of kind `schedule` (a run of show). Read with `getSchedule`. */
export interface ScheduleDoc {
  id: string;
  name: string;
  kind: "schedule";
  items: ScheduleItem[];
  /** A delay also shifts every later item. */
  ripple: boolean;
  /** The operator's pick for UP NEXT; null = automatic. */
  nextOverride: string | null;
  labels: ScheduleLabels;
  updatedAt: string;
}

/** Computed status (also `data-status` on air). */
export type ItemStatus = "previous" | "now" | "next" | "delayed" | "upcoming" | "cancelled";

/** Columns a schedule exposes to bindings; `countdown` is a timer state (bind it to a timer). */
export const SCHEDULE_COLUMNS: ReadonlyArray<{ key: string; label: string }> = [
  { key: "title", label: "Title" },
  { key: "subtitle", label: "Subtitle" },
  { key: "location", label: "Location" },
  { key: "start", label: "Start" },
  { key: "end", label: "End" },
  { key: "day", label: "Day" },
  { key: "new_start", label: "New start" },
  { key: "label", label: "Status label" },
  { key: "status", label: "Status" },
  { key: "countdown", label: "Countdown" },
];

export interface ScheduleItemView {
  id: string;
  title: string;
  location: string;
  status: ItemStatus;
  /** Effective local date-times (delays and actual times applied). */
  start: string;
  end: string;
  /** Server epoch ms. */
  startMs: number;
  endMs: number;
  /** The planned start differs (delayed or moved). */
  moved: boolean;
}

/** Payload of `schedule-changed`; reply of scheduleView / scheduleAction. */
export interface ScheduleView {
  sourceId: string;
  items: ScheduleItemView[];
  nowId: string | null;
  nextId: string | null;
  /** Server epoch ms when computed. */
  computedAt: number;
  canUndo: boolean;
}

export type ScheduleAction =
  | { do: "startNow"; itemId: string }
  | { do: "end"; itemId: string }
  | { do: "setNext"; itemId: string; start: string | null }
  | { do: "delay"; itemId: string; minutes: number }
  | { do: "setStart"; itemId: string; start: string }
  | { do: "cancel"; itemId: string }
  | { do: "restore"; itemId: string }
  | { do: "auto"; itemId: string }
  | { do: "undo" };

// ---- Assets (mirrors the Assets block in model.rs) -------------------------------------------

export const ASSET_EXTENSIONS = ["png", "jpg", "jpeg", "svg", "gif", "webp"] as const;
export const ASSET_MAX_BYTES = 20 * 1024 * 1024;

/** An image in the project's assets/ folder, served at `/assets/<name>`. */
export interface AssetInfo {
  name: string;
  /** The value image fields and cells hold: `/assets/<name>`. */
  url: string;
  bytes: number;
  modifiedAt: string;
  /** Screen and source ids that reference it. */
  usedBy: string[];
  /** Probed details for video and audio files (and image size). Absent for files not probed yet. */
  media?: MediaInfo;
}

/** Images ≤ 20 MB; video and audio ≤ 8 GiB. */
export const ASSET_IMAGE_MAX_BYTES = 20 * 1024 * 1024;
export const ASSET_MEDIA_MAX_BYTES = 8 * 1024 * 1024 * 1024;

export type MediaKind = "image" | "video" | "audio";

/** What the core read from the file's headers (`core/media_probe.rs`; no external tools). */
export interface MediaInfo {
  kind: MediaKind;
  /** mp4 (incl. m4a), webm, mov, mp3, wav, aac (ADTS), png, jpg, webp, gif, svg. */
  container: string;
  durationMs: number | null;
  width: number | null;
  height: number | null;
  fps: number | null;
  /** "h264", "hevc", "vp8", "vp9", "av1", "prores", … with profile when known ("h264 High 4.1"). */
  videoCodec: string | null;
  /** "aac", "mp3", "opus", "vorbis", "pcm", … */
  audioCodec: string | null;
  /** Video with an alpha channel (WebM AlphaMode, HEVC with alpha, ProRes 4444). */
  alpha: boolean;
  /** MP4/MOV: the index (moov) comes before the media data. Null for other containers. */
  fastStart: boolean | null;
  issues: MediaIssue[];
}

/** Where a file plays: the app's own WebView2 (Windows), WKWebView (macOS), OBS / vMix browser
 * sources. */
export type MediaPlatform = "windows" | "macos" | "obs" | "vmix";

export interface MediaIssue {
  code: "unsupported" | "noAlpha" | "needsExtension" | "slowStart" | "unreadable";
  /** error = won't play there; warning = plays with a caveat. */
  severity: "error" | "warning";
  platforms: MediaPlatform[];
  /** Operator copy, short ("Won't play in OBS: HEVC."). */
  message: string;
  /** A conversion that fixes it: a label ("Convert to H.264") and a copyable ffmpeg command line
   * for this file. Null when there is none. */
  convert: { label: string; command: string } | null;
}

/** `asset-import` events while importing (≤ 10/s per file). */
export interface AssetImportProgress {
  /** Import job id (from `import_media`'s reply). */
  job: string;
  /** The file's position in the picked paths: the stable key (its name in assets/ is sanitized
   * and may get a "-2" suffix, so it can differ from the picked file's name). */
  index: number;
  /** The file's name in assets/ (the picked name before it was placed or when it failed). */
  name: string;
  copied: number;
  total: number;
  done: boolean;
  /** Set when this file failed (too large, unknown type, cancelled, disk error). */
  error: string | null;
}

// ---- Recipes (mirrors the Recipes block in model.rs) -----------------------------------------

export type RecipePanel = "slideshow" | "schedule" | "countdown";

export interface RecipeInfo {
  id: string;
  name: string;
  description: string;
  panel: RecipePanel;
  version: number;
  /** Preset template ids it uses (gallery preview). */
  templates: string[];
}

export type RecipeScreen =
  | { kind: "existing"; screenId: string }
  | { kind: "new"; name: string; width: number; height: number };

export interface InstallRecipe {
  recipeId: string;
  name: string;
  screen: RecipeScreen;
}

export interface RecipeInstance {
  id: string;
  recipeId: string;
  name: string;
  panel: RecipePanel;
  sourceId: string | null;
  screenId: string;
  layerIds: string[];
  templateIds: string[];
  /** The flows it created, and flows added to it later ("New rule" in its panel). */
  flowIds: string[];
  createdAt: string;
}

// ---- Flows (<project>/flows/<id>.json; mirrors core/model.rs) --------------------------------
//
// Port ids: event input `in`; triggers fire `fired`; actions `done` / `failed`; logic `out`
// unless: branch `then`/`else`; switch `case0`.. + `default`; gate in `in`/`open`/`close`; delay in
// `in`/`cancel`; waitUntil in `in`/`release`, out `out`/`timeout`; once/counter in `in`/`reset`;
// sequence out `s0`..`s7`; random out `r0`..`r7`. Data ports are named after their value (see
// core/model.rs).

export const FLOWS_MAX = 128;
export const FLOW_NODES_MAX = 200;
export const FLOW_EDGES_MAX = 400;
export const FLOW_NAME_MAX = 100;
export const FLOW_DELAY_MAX_MS = 86_400_000;
export const FLOW_CASES_MAX = 16;
export const FLOW_BRANCHES_MAX = 8;

export type Retrigger = "restart" | "ignore";
export type WireKind = "event" | "data";
export type DataType = "text" | "number" | "bool" | "time" | "row" | "rows";
export type RowPick =
  | { pick: "current" }
  | { pick: "first" }
  | { pick: "last" }
  | { pick: "index"; index: number };
export type NowNext = "now" | "next";
export type FlowConstant =
  | { kind: "text"; value: string }
  | { kind: "number"; value: number }
  | { kind: "bool"; value: boolean };
export type MathOp = "add" | "sub" | "mul" | "div" | "min" | "max" | "round";
export type CompareOp = "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "contains" | "empty";
export type PauseMode = "pause" | "resume" | "toggle";
export type ItemPick =
  | { pick: "now" }
  | { pick: "next" }
  | { pick: "item"; itemId: string }
  | { pick: "input" };
export type ScheduleOp =
  | { op: "startNow" }
  | { op: "end" }
  | { op: "setNext"; start?: string | null }
  | { op: "delay"; minutes: number }
  | { op: "setStart"; start: string }
  | { op: "cancel" }
  | { op: "restore" }
  | { op: "auto" };

/** A node's `type` + `config` (`core/flow_nodes.rs`). */
export type NodeKind =
  // Triggers
  | { type: "trigger.cue"; config: { label?: string | null } }
  | {
      type: "trigger.itemStatus";
      config: { sourceId: string; status: ItemStatus; itemId?: string | null };
    }
  | { type: "trigger.beforeNext"; config: { sourceId: string; minutes: number } }
  | { type: "trigger.timeOfDay"; config: { at: string; days: number[] } }
  | { type: "trigger.timerZero"; config: { screenId: string; layerId: string; key: string } }
  | { type: "trigger.layer"; config: { screenId: string; layerId: string; onAir: boolean } }
  | { type: "trigger.slideshowWrap"; config: { sourceId: string } }
  | { type: "trigger.sourceRow"; config: { sourceId: string } }
  | { type: "trigger.runByFlow"; config: Record<string, never> }
  /** An API source's data changed: with `column`, that column of the current row (● value: text);
   * without, any row (● row). */
  | { type: "trigger.apiChanged"; config: { sourceId: string; column?: string | null } }
  /** MIDI in: a note-on (velocity > 0) or a CC change from `device` (null = any) on `channel`
   * (1–16, null = any); ● value: number (0–127, the velocity or CC value). */
  | {
      type: "trigger.midi";
      config: { device: string | null; channel: number | null; kind: "note" | "cc"; number: number };
    }
  /** OSC in: a message whose address matches `address` (exact, `*` = one path segment);
   * ● value: text (the first argument as text). */
  | { type: "trigger.osc"; config: { address: string } }
  /** A media layer reached its out point with end action hold or clear (not loop). */
  | { type: "trigger.mediaEnded"; config: { screenId: string; layerId: string } }
  // Data
  | { type: "data.sourceRow"; config: { sourceId: string; row: RowPick } }
  | { type: "data.scheduleItem"; config: { sourceId: string; which: NowNext } }
  | { type: "data.fieldValue"; config: { screenId: string; layerId: string; key: string } }
  | { type: "data.layerState"; config: { screenId: string; layerId: string } }
  | { type: "data.constant"; config: { value: FlowConstant } }
  | { type: "data.timeNow"; config: Record<string, never> }
  | { type: "data.format"; config: { pattern: string } }
  | { type: "data.math"; config: { op: MathOp; a: number; b: number } }
  | { type: "data.compare"; config: { op: CompareOp; a: string; b: string } }
  // Logic
  | { type: "logic.branch"; config: Record<string, never> }
  | { type: "logic.switch"; config: { cases: string[] } }
  | { type: "logic.gate"; config: { open: boolean } }
  | { type: "logic.delay"; config: { ms: number } }
  | { type: "logic.waitUntil"; config: { at?: string | null; timeoutMs?: number | null } }
  | { type: "logic.debounce"; config: { ms: number } }
  | { type: "logic.throttle"; config: { ms: number } }
  | { type: "logic.sequence"; config: { count: number } }
  | { type: "logic.once"; config: Record<string, never> }
  | { type: "logic.counter"; config: { every: number } }
  | { type: "logic.random"; config: { count: number; noRepeat: boolean } }
  /**
   * Loop: event in `in` (start), `stop`; event out `body` (each round; the next round starts
   * once the body's branch has finished, incl. its delays, + `delayMs`), `done` (after the last
   * round or a `stop`); data out `round` (number, from 1). `count` 1..=10000 or null = forever.
   * Rounds start at most every 100 ms (paced, with a debug warning); 256 node runs per round.
   */
  | { type: "logic.repeat"; config: { count: number | null; delayMs: number } }
  // Actions
  | {
      type: "action.take";
      config: { screenId: string; layerId: string; forMs?: number | null };
    }
  | { type: "action.clear"; config: { screenId: string; layerId: string } }
  | { type: "action.clearScreen"; config: { screenId: string } }
  | { type: "action.clearAll"; config: Record<string, never> }
  | {
      type: "action.setFields";
      config: { screenId: string; layerId: string; fields: Record<string, string> };
    }
  | {
      type: "action.setTimer";
      config: { screenId: string; layerId: string; key: string; action: TimerAction };
    }
  | { type: "action.stepSource"; config: { sourceId: string; step: SourceStep } }
  | { type: "action.pauseSource"; config: { sourceId: string; mode: PauseMode } }
  | { type: "action.schedule"; config: { sourceId: string; item: ItemPick; op: ScheduleOp } }
  | { type: "action.runFlow"; config: { flowId: string; wait: boolean } }
  /** Studio mode: takes the screen's Preview to Program (transition null = the project default). */
  | { type: "action.takePreview"; config: { screenId: string; transition: TakeTransition | null } }
  /** Stops a flow's running instance (any flow, including this one: then this instance ends
   * after the node and `done` doesn't fire). Not running = nothing to do, `done`. */
  | { type: "action.stopFlow"; config: { flowId: string } }
  | {
      type: "action.setVariant";
      config: { screenId: string; layerId: string; variant: string; transition: VariantTransition };
    }
  | { type: "action.log"; config: { message: string; toast: boolean } }
  /** Media transport on a media layer's Program playback (`seek`: to `ms`, clamped to the range). */
  | {
      type: "action.media";
      config: { screenId: string; layerId: string; action: MediaFlowAction; ms?: number | null };
    };

export type MediaFlowAction = "play" | "pause" | "stop" | "restart" | "seek";

export type NodeType = NodeKind["type"];

export type FlowNode = NodeKind & { id: string; position: { x: number; y: number } };

export interface PortRef {
  node: string;
  port: string;
}

export interface FlowEdge {
  id: string;
  from: PortRef;
  to: PortRef;
  kind: WireKind;
}

export interface FlowDraft {
  nodes: FlowNode[];
  edges: FlowEdge[];
  updatedAt: string;
}

/** `flows/<id>.json`: `nodes`/`edges` are the deployed graph. */
export interface Flow {
  id: string;
  name: string;
  armed: boolean;
  retrigger: Retrigger;
  /** The recipe instance that created it. */
  origin: string | null;
  nodes: FlowNode[];
  edges: FlowEdge[];
  /** Undeployed editor changes; absent when none. */
  draft?: FlowDraft | null;
  lastFiredAt: string | null;
  updatedAt: string;
}

export interface FlowIssue {
  node: string | null;
  message: string;
}

/** A flow as listed: the file plus server-side state. */
export interface FlowInfo extends Flow {
  warnings: FlowIssue[];
  running: boolean;
  readOnly: boolean;
}

export interface NewFlow {
  name: string;
  armed: boolean;
  retrigger: Retrigger;
  /** An installed recipe instance id: the flow belongs to that recipe (added to its flowIds). */
  origin?: string | null;
  nodes: FlowNode[];
  edges: FlowEdge[];
}

export interface SaveFlow {
  id: string;
  name: string;
  retrigger: Retrigger;
  nodes: FlowNode[];
  edges: FlowEdge[];
}

export type FlowRun = "running" | "finished" | "stopped" | "failed";

/** Payload of `flow-state` (replaces `macro-state` and `recipe-state`). */
export interface FlowState {
  id: string;
  state: FlowRun;
  node: string | null;
  message: string | null;
  /** RFC 3339; for `running` also the flow's new `lastFiredAt`. */
  at: string;
}

/** Payload of `flow-log`: a Log action asked for a toast (≤ 1 per 2 s per flow). */
export interface FlowLog {
  id: string;
  node: string;
  message: string;
  at: string;
}

/** A node that ran, in a `flow-debug` batch. */
export interface FlowDebugNode {
  node: string;
  /** RFC 3339 of its last run in this batch. */
  at: string;
}

/** Payload of `flow-debug` (≤ 10 batches/s per flow, only while subscribed). */
export interface FlowDebug {
  id: string;
  nodes: FlowDebugNode[];
  /** Event edges an event travelled along. */
  edges: string[];
  /** Run-time errors per node. */
  errors: FlowIssue[];
}

// ---- Flow templates (built-in, presets/flows/<id>.json) ----

export type FlowTemplateCategory = "show" | "schedule" | "sponsors" | "sports" | "blocks";
/** Slot values: a screen/source/flow id; a layer "screenId/layerId"; a timer or field
 * "screenId/layerId/key". */
export type FlowSlotKind =
  | "screen"
  | "layer"
  | "textSource"
  | "scheduleSource"
  | "timer"
  | "field"
  | "flow";

export interface FlowSlot {
  key: string;
  label: string;
  kind: FlowSlotKind;
  /** Prefer layers whose template came from this preset. */
  prefer: { template?: string | null };
  optional: boolean;
}

export interface FlowTemplateInfo {
  id: string;
  name: string;
  description: string;
  category: FlowTemplateCategory;
  /** e.g. "Cue → Take → Delay → Clear layer". */
  chain: string;
  slots: FlowSlot[];
  /** How many flows adding it creates. */
  flows: number;
}

export interface AddFlowTemplate {
  templateId: string;
  name?: string | null;
  slots: Record<string, string>;
}

/** Result of `check_flow`: `errors` block deploy, `warnings` don't. */
export interface FlowCheck {
  errors: FlowIssue[];
  warnings: FlowIssue[];
}

// ---- Remote control (src-tauri/src/core/remote; not in model.rs, mirrored here) ---------------

/** Where remote control is (`RemoteView.state`). */
export type RemotePhase =
  | { state: "off" }
  | { state: "connecting" }
  | { state: "live"; runId: string; resumed: boolean }
  | { state: "reconnecting"; attempt: number; nextMs: number }
  /** The show runs on another machine: Move it here / Make this a separate show / Cancel. */
  | { state: "showInUse"; machine: string }
  /** The run ended; the switch stays on: Start again. */
  | { state: "ended" }
  /** Stopped for something the user handles (sign in, plan, update, clock, moved, limits). */
  | { state: "stopped"; code: string; message: string };

/** Someone connected through the control site (names are shown to the owner only). */
export interface CrewMember {
  id: string | null;
  name: string;
  /** viewer | data_editor | operator */
  role: string;
  anonymous: boolean;
  sinceMs: number;
}

/** `remote_view` and the `remote-control` event. */
export type RemoteView = RemotePhase & {
  /** The switch for the open project (this session). */
  on: boolean;
  takeLocked: boolean;
  crew: CrewMember[];
  showId: string | null;
  crewUrl: string | null;
  controllerUrl: string | null;
  serverVersion: string | null;
  rttMs: number | null;
  licenseWarning: { endsMs: number; message: string } | null;
  lastError: string | null;
  /** The project has a remote control ID ("Make this a separate show" is offered). */
  hasShowId: boolean;
};

/** An accepted remote command (`remote-activity`; this run only). */
export interface RemoteActivity {
  at: number;
  name: string;
  role: string;
  /** "Kari N. updated Name on Lower third 2". */
  text: string;
}

/** The cue list's position (`cue_position`, `cue-position`): Control marks `next`. */
export interface CuePosition {
  current: string | null;
  next: string | null;
}

/** Test connection. */
export interface RemoteTestResult {
  ok: boolean;
  /** ok, signed_out, offline, timeout, unauthorized, rate_limited, server_busy, … */
  code: string;
  message: string;
  detail: string | null;
  serverVersion: string | null;
  connectMs: number | null;
  welcomeMs: number | null;
  rttMs: number | null;
  retryAfterS: number | null;
  /** Measured on the live connection (remote control on). */
  live: boolean;
}

/** Connection details (`remote_diag_watch`, `remote-diag`): the diagnostics export's shape. */
export interface RemoteDiag {
  serverVersion: string | null;
  state: string;
  runId: string | null;
  showId: string | null;
  protocol: number;
  rtt: { last: number | null; median: number | null; max: number | null };
  clockOffsetMs: number | null;
  connects: number;
  commands: { received: number; accepted: number; resultMsMedian: number | null; resultMsMax: number | null };
  counters: Record<string, { in: number; inBytes: number; out: number; outBytes: number }>;
  snapshot: { seq: number | null; bytes: number | null; sent: number; resyncs: number; lastResync: { reason: string; corr: string } | null };
  lastClose: { code: number; reason: string } | null;
  reconnects: { at: number; closeCode?: number; reason: string; backoffMs: number }[];
  refused: { at: number; corr: string; action: string; role: string; code: string; layer?: string; cueList?: string; cue?: string }[];
  errors: { at: number; code: string; fatal: boolean; corr?: string }[];
  probe: { at: number; outcome: string; connectMs?: number; welcomeMs?: number; rttMs?: number } | null;
}
