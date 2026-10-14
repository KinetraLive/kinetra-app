// Typed wrappers around Tauri `invoke`, one per command in src-tauri/src/commands.
// Components never call `invoke` directly.
//
// This is the core module, imported by the root layout and therefore part of every route's initial
// bundle: `call`, errors, events, project, recents, prefs and UI state. Domain commands
// live in sibling modules so a route loads only what it uses: `$lib/api/templates`, `$lib/api/screens`, `$lib/api/sources`, `$lib/api/automation`, `$lib/api/recipes`, `$lib/api/flows`, `$lib/api/assets`,
// `$lib/api/live`, `$lib/api/backups`, `$lib/api/settings-file`, `$lib/api/move`, `$lib/api/reset`,
// `$lib/api/app` (app info, project settings, clearing recents, the native menu, zoom, updates).

import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppError,
  BackupInfo,
  FilesChanged,
  FlowDebug,
  FlowLog,
  FlowState,
  LiveScreenView,
  ScheduleView,
  FolderKind,
  MoveProgress,
  PollLive,
  Prefs,
  Project,
  RecentProject,
  ServerStatus,
  SourceSummary,
  UiState,
} from "../types";

/** Normalizes anything a command can reject with into an `AppError`. */
export function toAppError(e: unknown): AppError {
  if (e && typeof e === "object" && "code" in e && "message" in e) return e as AppError;
  return { code: "internal", message: e instanceof Error ? e.message : String(e) };
}

// The Tauri API modules load on first use instead of statically. They are shared with the window
// API that the root layout loads right after first paint, so this moves them off the critical path
// of every route (picker budget) at the cost of one cached dynamic import.
const tauriCore = () => import("@tauri-apps/api/core");

export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const { invoke } = await tauriCore();
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toAppError(e);
  }
}

// ---- App ----------------------------------------------------------------------------------
/** Opens (or focuses) the Preferences window at a section (an id or an old /settings anchor). */
export const openPreferences = (section?: string) =>
  call<void>("open_preferences", { section: section ?? null });

// ---- Project ------------------------------------------------------------------------------
export const inspectFolder = (path: string) => call<FolderKind>("inspect_folder", { path });
export const createProject = (parent: string, name: string) =>
  call<Project>("create_project", { parent, name });
export const initProject = (path: string, name?: string) =>
  call<Project>("init_project", { path, name: name ?? null });
export const openProject = (path: string) => call<Project>("open_project", { path });
export const closeProject = () => call<void>("close_project");
export const currentProject = () => call<Project | null>("current_project");

// ---- Recents ------------------------------------------------------------------------------
export const listRecents = () => call<RecentProject[]>("list_recents");
export const removeRecent = (path: string) => call<RecentProject[]>("remove_recent", { path });

// ---- Prefs / UI state ---------------------------------------------------------------------
export const getPrefs = () => call<Prefs>("get_prefs");
export const setPrefs = (prefs: Prefs) => call<Prefs>("set_prefs", { prefs });
export const getUiState = () => call<UiState>("get_ui_state");
export const setUiState = (uiState: UiState) => call<UiState>("set_ui_state", { uiState });

// ---- Events -------------------------------------------------------------------------------
// Mirrors src-tauri/src/core/events.rs `names`.
export interface EventPayloads {
  "recents-checked": RecentProject[];
  "project-move-progress": MoveProgress;
  "store-recovered": { file: string; quarantinedAs: string };
  "backup-created": BackupInfo;
  "background-error": { message: string };
  "server-status": ServerStatus;
  "project-files-changed": FilesChanged;
  /** A screen's live view changed (on air, pending, fields, layers), from any path. */
  "live-changed": LiveScreenView;
  /** A source changed without a command reply to carry it (auto-advance step, arming). */
  "source-changed": SourceSummary;
  /** A poll's latest data or how it arrives (polls with a stream or a watcher; ≤ 4/s). */
  "poll-live": PollLive;
  "schedule-changed": ScheduleView;
  /** A flow started, finished, stopped or failed (replaces macro-state and recipe-state). */
  "flow-state": FlowState;
  /** A flow's Log action asked for a toast (≤ 1 per 2 s per flow); shown app-wide by the shell. */
  "flow-log": FlowLog;
  "flow-debug": FlowDebug;
  /** A screen's Preview changed (Studio mode): cues, staged content, T-bar position. */
  "preview-changed": import("../types").PreviewView;
  // MIDI / OSC control (wrappers and subscriptions: $lib/api/midi).
  /** The MIDI devices changed (plugged in, unplugged, opened). */
  "midi-devices": import("../types").MidiDevice[];
  /** A MIDI message captured while a learn session is open. */
  "midi-learn": import("../types").MidiLearned;
  /** controls.json changed (a save, an external edit). */
  "controls-changed": import("../types").ControlsDoc;
  /** An input matched two or more mappings: none ran (≤ 1 per 2 s per source). */
  "control-conflict": { source: import("../types").MidiSource; mappingIds: string[] };
  /** The OSC listener started, stopped or couldn't bind its port. */
  "osc-status": import("../types").OscStatus;
  // Windows: emitted to every app window except the one whose command caused it, so the main
  // window and the Preferences window stay in sync without echoing their own optimistic writes.
  /** prefs.json changed (set_prefs, set_zoom, reset, import). */
  "prefs-changed": Prefs;
  /** ui-state.json changed (set_ui_state, import, project open/close: the new project's state). */
  "ui-state-changed": UiState;
  /** The open project or its settings changed (open, close, settings, move, brand kit). */
  "project-changed": Project | null;
  /** Main window: a native menu item was clicked; run the registry command `id`. */
  menu: { id: string };
  /** Main window: its close button was pressed with a project open. Answer with
   * `closeRequestAck` ($lib/api/app), then run File › Exit (the menu sync does). */
  "close-requested": null;
  /** Preferences window: `open_preferences` asked for a section while the window was open. */
  "prefs-section": { section: string };
  /** Installing an update. */
  "update-progress": import("../types").UpdateProgress;
  // Display outputs (wrappers: $lib/api/outputs).
  /** The connected displays changed (plugged in, unplugged, rearranged), or a watch started. */
  "displays-changed": import("../types").DisplayInfo[];
  /** An output's status changed (shown, display missing, screen missing, pattern on/off). */
  "outputs-changed": import("../types").OutputStatus[];
  /** The license, sign-in progress or account changed (display only). Wrappers: $lib/api/license. */
  "license-changed": import("../types").LicenseView;
  /** An AI job started, ended or was canceled. Wrappers: $lib/api/ai. */
  "ai-job": import("../types").AiJobView;
  // Media (wrappers: $lib/api/media).
  /** Import progress per file (≤ 10/s). */
  "asset-import": import("../types").AssetImportProgress;
  // Remote control (wrappers: $lib/api/remote).
  /** Remote control's state, crew, lock or round trip changed. */
  "remote-control": import("../types").RemoteView;
  "control-api": import("../types").ControlApiView;
  /** A remote command was accepted (this run only). */
  "remote-activity": import("../types").RemoteActivity;
  /** Connection details, ≤ 2/s and only while a watch is open. */
  "remote-diag": import("../types").RemoteDiag;
  /** The cue list's current / next cue changed (Control marks Next). */
  "cue-position": import("../types").CuePosition;
}

/**
 * Subscribes to a backend event. Returns a synchronous unsubscribe so it can be returned straight
 * from `$effect` / `onMount`; it is safe to call before `listen` has resolved.
 * `@tauri-apps/api/event` loads on first use (with ./listen), like the core module above.
 */
export function on<E extends keyof EventPayloads>(
  event: E,
  handler: (payload: EventPayloads[E]) => void,
): () => void {
  let unlisten: UnlistenFn | undefined;
  let cancelled = false;
  // Listens as this window (listen.ts, lazy with the event module).
  import("./listen")
    .then(({ listenHere }) => {
      if (cancelled) return;
      return listenHere<EventPayloads[E]>(event, handler).then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    })
    // No Tauri runtime (plain browser, tests): there is nothing to listen to.
    .catch(() => {});
  return () => {
    cancelled = true;
    unlisten?.();
  };
}
