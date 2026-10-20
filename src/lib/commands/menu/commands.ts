// The app-level commands behind the native menu (File, Edit, View, Show, Output, Help) that no page owns.
// Registered by the menu sync (./sync.svelte.ts, main window only) for the whole session, so they
// are in the palette and Keyboard shortcuts too; Preferences lists inert copies from `appCommands`.
//
//   app.newProject      New project            Ctrl+N   picker: its own form; in a project: back to it
//   app.openProject     Open project           Ctrl+O   picker: its folder dialog; in a project: the
//                                                        top-bar switcher's Open… (OPEN_PROJECT_EVENT)
//   app.openRecent:<p>  Open {project}         provider "app.recents" (File › Open recent)
//   app.clearRecents    Clear recent projects
//   app.importSettings  Import settings        Preferences › Advanced › Settings file
//   app.exportSettings  Export settings        save dialog, then export_settings
//   app.preferences     Preferences            hidden ("Go to Preferences" is the palette's); the
//                                              menu shows that command's keys (Ctrl+,)
//   app.exit            Exit Kinetra           asks first while anything is on air
//   edit.undo / .redo   Undo / Redo            hidden; Windows menu only (see `editorUndo`)
//   view.simple / .advanced   Simple / Advanced mode
//   view.zoomIn / .zoomOut / .actualSize   Ctrl+= / Ctrl+- / Ctrl+0 (WebView zoom, per machine)
//   view.fullscreen     Full screen            F11 (in Control only while no layer or cue key uses it)
//   show.take / .clear  Take / Clear the selected layer (Control: the one expanded layer row;
//                       Studio mode: Take = "Take preview"); hidden, Control's provider lists the
//                       explicit actions
//   output.displays     Displays            hidden (the palette has "Preferences: Outputs"); Output › Displays…
//   output.toggle:<id>  Turn {output} on / off   provider "app.outputs" (Output menu, checkable)
//   output.allOff       All outputs off      every output off (Output menu)
//   help.docs           Documentation          https://docs.kinetra.live
//   help.shortcuts      Keyboard shortcuts     hidden (Preferences: Keyboard shortcuts is listed)
//   help.updates        Check for updates      Preferences › About, starting a check
//   help.about          About Kinetra          Preferences › About

import { goto } from "$app/navigation";
import { loadQuitCheck } from "$cloud";
import { openPreferences, toAppError } from "$lib/api";
import { clearRecents, exitApp, setZoom, toggleFullscreen } from "$lib/api/app";
import { listScreens } from "$lib/api/live";
import { allOutputsOff, listOutputs } from "$lib/api/outputs";
import { menuLabel, patchOutput, showingText } from "$lib/components/outputs/labels";
import { CHECK_UPDATES } from "$lib/components/settings/sections";
import { sectionRoot } from "$lib/components/shell/nav";
import { t } from "$lib/i18n";
import { setUiMode } from "$lib/stores/mode.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { project, uiState } from "$lib/stores/project.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { ZOOM_MAX, ZOOM_MIN, ZOOM_STEP } from "$lib/types";
import { keysFor } from "../bindings.svelte";
import { registry, type Command, type CommandContext, type CommandProvider } from "../registry.svelte";

export const DOCS_URL = "https://docs.kinetra.live";
/** Window event the top-bar project switcher answers with its "Open…" flow. */
export const OPEN_PROJECT_EVENT = "kinetra:open-project";
export const RECENT_PREFIX = "app.openRecent:";

const onPicker = () => location.pathname === "/";

/** The picker's own Ctrl+N / Ctrl+O handlers (it has no shell, so no registry keys). */
function pickerKey(key: "n" | "o") {
  window.dispatchEvent(new KeyboardEvent("keydown", { key, code: `Key${key.toUpperCase()}`, ctrlKey: true, bubbles: true, cancelable: true }));
}

async function newProject() {
  if (onPicker()) return pickerKey("n");
  // Navigate first (like the switcher): the shell's "no project → /" guard can't drop `?new=1`.
  await goto("/?new=1");
  await project.close();
}

function openProject() {
  if (onPicker()) pickerKey("o");
  else window.dispatchEvent(new CustomEvent(OPEN_PROJECT_EVENT));
}

async function openRecent(path: string, name: string) {
  const from = location.pathname;
  try {
    await project.open(path);
  } catch (e) {
    toasts.push({ kind: "error", message: t("Couldn't open {project}: {reason}", { project: name, reason: toAppError(e).message }) });
    return;
  }
  const target = from === "/" ? "/dashboard" : sectionRoot(from);
  if (target !== location.pathname) await goto(target);
}

async function clearRecentProjects() {
  await clearRecents();
  project.recents = [];
}

async function exportSettingsFile() {
  const [{ save }, { exportSettings }] = await Promise.all([import("@tauri-apps/plugin-dialog"), import("$lib/api/settings-file")]);
  const path = await save({ title: t("Export settings"), defaultPath: "kinetra-settings.json", filters: [{ name: t("Settings"), extensions: ["json"] }] });
  if (!path) return;
  try {
    await exportSettings(path);
    toasts.push({ kind: "success", message: t("Settings exported to {path}", { path }) });
  } catch (e) {
    toasts.push({ kind: "error", message: t("Couldn't export: {reason}", { reason: toAppError(e).message }) });
  }
}

/** True when any screen has a layer on air (Program). */
async function anythingOnAir(): Promise<boolean> {
  if (!project.current) return false;
  try {
    return (await listScreens()).some((v) => v.layers.some((l) => l.onAir));
  } catch {
    return false;
  }
}

/** How many display outputs are showing (0 when they can't be read). */
async function outputsShowing(): Promise<number> {
  if (!prefs.value?.outputs?.length) return 0;
  try {
    return (await listOutputs()).filter((s) => s.state === "showing").length;
  } catch {
    return 0;
  }
}

async function exit() {
  const [onAir, outputs] = await Promise.all([anythingOnAir(), outputsShowing()]);
  if (onAir || outputs > 0) {
    const { ask } = await import("@tauri-apps/plugin-dialog");
    const lines = [onAir ? t("Some graphics are still on air.") : "", outputs > 0 ? t(showingText(outputs)) + " " + t("Exit anyway?") : ""];
    const ok = await ask(lines.filter(Boolean).join(" "), {
      title: t("Exit Kinetra?"),
      kind: "warning",
      okLabel: t("Exit"),
      cancelLabel: t("Cancel"),
    });
    if (!ok) return;
  } else {
    // Cloud backup may ask first (only with changes not in the cloud; never while on air; only in
    // builds with cloud backup).
    if (loadQuitCheck) {
      const backUpBeforeQuit = await loadQuitCheck();
      if (await backUpBeforeQuit(exitApp)) return;
    }
  }
  await exitApp();
}

// ---- Zoom ----------------------------------------------------------------------------------------

/** A zoom level clamped to 0.5–2 and rounded to 0.1. */
export const clampZoom = (z: number): number => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(z * 10) / 10));
const zoom = () => prefs.value?.zoom ?? 1;

async function zoomTo(z: number) {
  const next = clampZoom(z);
  if (next === zoom()) return;
  const applied = await setZoom(next);
  // The backend tells the other windows; this one updates its copy itself.
  const p = prefs.value;
  if (p) {
    const { zoom: _old, ...rest } = p;
    prefs.set(applied === 1 ? rest : { ...rest, zoom: applied });
  }
}

// ---- Undo / Redo -----------------------------------------------------------------------------------

/**
 * Edit › Undo / Redo (Windows; macOS uses the predefined items): a text field undoes its own
 * typing; anything else gets the editor's own key (Ctrl+Z / Ctrl+Shift+Z) dispatched at the focused
 * element, so it goes the same way the keys do: the flow editor's undo (through the registry),
 * CodeMirror and the sheet editor (their own keymaps).
 */
export function editorUndo(redo: boolean): void {
  const el = document.activeElement;
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
    document.execCommand(redo ? "redo" : "undo");
    return;
  }
  const target = el instanceof HTMLElement ? el : document.body;
  target.dispatchEvent(new KeyboardEvent("keydown", { key: redo ? "Z" : "z", code: "KeyZ", ctrlKey: true, shiftKey: redo, bubbles: true, cancelable: true }));
}

// ---- Show ----------------------------------------------------------------------------------------

/** Control's selected layer: the selected screen and its one expanded layer row. */
function selectedLayer(ctx: CommandContext): string | null {
  if (ctx.route !== "control" || !ctx.screenId || ctx.layerIds.length !== 1) return null;
  return `${ctx.screenId}/${ctx.layerIds[0]}`;
}

/** Control's palette item for the selected layer (`live.take:<s>/<l>`, …), first of `kinds`. */
function liveItem(ctx: CommandContext, kinds: readonly string[]): Command | undefined {
  const sel = selectedLayer(ctx);
  if (!sel) return undefined;
  const items = registry.providers.find((p) => p.id === "control.live")?.items(ctx) ?? [];
  for (const k of kinds) {
    const c = items.find((i) => i.id === `${k}:${sel}`);
    if (c) return c;
  }
  return undefined;
}

const registered = (id: string) => registry.list.find((c) => c.id === id);
const TAKE = ["live.update", "live.take"] as const;
const CLEAR = ["live.clear"] as const;

function takeBlocked(ctx: CommandContext): string | null {
  if (ctx.studio) {
    const s = registered("studio.take");
    return s ? (s.disabled?.(ctx) ?? null) : t("Select a screen first");
  }
  if (!selectedLayer(ctx)) return t("Select a layer first");
  return liveItem(ctx, TAKE) ? null : t("Already on air");
}

function clearBlocked(ctx: CommandContext): string | null {
  if (ctx.studio) return t("In Studio mode, cue layers out in Preview");
  if (!selectedLayer(ctx)) return t("Select a layer first");
  return liveItem(ctx, CLEAR) ? null : t("Not on air");
}

// ---- The commands --------------------------------------------------------------------------------

export function appCommands(): Command[] {
  const inProject = (ctx: CommandContext) => ctx.project;
  return [
    { id: "app.newProject", title: "New project", section: "create", defaultKeys: ["Ctrl+N"], run: newProject },
    { id: "app.openProject", title: "Open project", section: "goto", defaultKeys: ["Ctrl+O"], run: openProject },
    {
      id: "app.clearRecents",
      title: "Clear recent projects",
      section: "settings",
      noRecent: true,
      disabled: () => (project.recents.some((r) => r.path !== project.current?.path) ? null : t("No recent projects")),
      run: clearRecentProjects,
    },
    { id: "app.importSettings", title: "Import settings", section: "settings", when: inProject, run: () => openPreferences("settings-file") },
    { id: "app.exportSettings", title: "Export settings", section: "settings", when: inProject, run: exportSettingsFile },
    { id: "app.preferences", title: "Preferences", section: "goto", hidden: true, run: () => openPreferences() },
    { id: "app.exit", title: "Exit Kinetra", section: "settings", noRecent: true, run: exit },
    { id: "edit.undo", title: "Undo", section: "edit", hidden: true, noRecent: true, run: () => editorUndo(false) },
    { id: "edit.redo", title: "Redo", section: "edit", hidden: true, noRecent: true, run: () => editorUndo(true) },
    { id: "view.simple", title: "Simple mode", section: "settings", scope: "setting", run: () => setUiMode("simple") },
    { id: "view.advanced", title: "Advanced mode", section: "settings", scope: "setting", run: () => setUiMode("advanced") },
    {
      id: "view.zoomIn",
      title: "Zoom in",
      section: "settings",
      defaultKeys: ["Ctrl+="],
      repeatable: true,
      disabled: () => (zoom() >= ZOOM_MAX ? t("Largest size") : null),
      run: () => zoomTo(zoom() + ZOOM_STEP),
    },
    {
      id: "view.zoomOut",
      title: "Zoom out",
      section: "settings",
      defaultKeys: ["Ctrl+-"],
      repeatable: true,
      disabled: () => (zoom() <= ZOOM_MIN ? t("Smallest size") : null),
      run: () => zoomTo(zoom() - ZOOM_STEP),
    },
    { id: "view.actualSize", title: "Actual size", section: "settings", defaultKeys: ["Ctrl+0"], run: () => zoomTo(1) },
    {
      id: "view.fullscreen",
      title: "Full screen",
      section: "settings",
      defaultKeys: ["F11"],
      // Control's layer and cue keys may use F11: there they win.
      when: (ctx) => ctx.route !== "control" || !hotkeyTakesFullscreen(),
      run: async () => void (await toggleFullscreen()),
    },
    {
      id: "show.take",
      title: "Take the selected layer",
      section: "live",
      keyScope: "control",
      live: true,
      hidden: true,
      noRecent: true,
      when: (ctx) => ctx.route === "control",
      disabled: takeBlocked,
      run: (ctx) => (ctx.studio ? registered("studio.take")?.run?.(ctx) : liveItem(ctx, TAKE)?.run?.(ctx)),
    },
    {
      id: "show.clear",
      title: "Clear the selected layer",
      section: "live",
      keyScope: "control",
      live: true,
      hidden: true,
      noRecent: true,
      when: (ctx) => ctx.route === "control",
      disabled: clearBlocked,
      run: (ctx) => liveItem(ctx, CLEAR)?.run?.(ctx),
    },
    { id: "output.displays", title: "Displays", section: "settings", hidden: true, run: () => openPreferences("outputs") },
    {
      id: "output.allOff",
      title: "All outputs off",
      section: "settings",
      noRecent: true,
      disabled: () => (prefs.value?.outputs?.some((o) => o.enabled) ? null : t("No output is on")),
      run: outputsOff,
    },
    { id: "help.docs", title: "Documentation", section: "goto", run: () => openUrl(DOCS_URL) },
    { id: "help.shortcuts", title: "Keyboard shortcuts", section: "settings", hidden: true, run: () => openPreferences("shortcuts") },
    { id: "help.updates", title: "Check for updates", section: "settings", run: () => openPreferences(CHECK_UPDATES) },
    { id: "help.about", title: "About Kinetra", section: "settings", run: () => openPreferences("about") },
  ];
}

/** A layer or cue key of this project is one of Full screen's keys (in Control those win). */
function hotkeyTakesFullscreen(): boolean {
  const mine = keysFor("view.fullscreen");
  return Object.values(uiState.value?.control.hotkeys ?? {}).some((k) => mine.includes(k));
}

async function openUrl(url: string) {
  const { openUrl: open } = await import("@tauri-apps/plugin-opener");
  await open(url);
}

// ---- Outputs -------------------------------------------------------------------------------------

export const OUTPUT_PREFIX = "output.toggle:";

async function outputsOff() {
  await allOutputsOff();
  // The core turned them off in prefs.json; this window reads it back (the event skips its sender).
  await prefs.load();
}

async function toggleOutput(id: string) {
  const p = prefs.value;
  const o = p?.outputs?.find((x) => x.id === id);
  if (!p?.outputs || !o) return;
  await prefs.update({ ...p, outputs: patchOutput(p.outputs, id, { enabled: !o.enabled }) });
}

/** The Output menu's items (and the palette's "Turn Output 1 on"): one per configured output. */
export const outputsProvider: CommandProvider = {
  id: "app.outputs",
  items(): Command[] {
    return (prefs.value?.outputs ?? []).map(
      (o): Command => ({
        id: OUTPUT_PREFIX + o.id,
        title: o.enabled ? "Turn {output} off" : "Turn {output} on",
        params: () => ({ output: o.name }),
        section: "settings",
        keywords: [menuLabel(o)],
        noRecent: true,
        run: () => toggleOutput(o.id),
      }),
    );
  },
};

// ---- Open recent ---------------------------------------------------------------------------------

/** File › Open recent (and the palette's "Open {project}" items): the recent projects other than
 * the open one, newest first, at most 10. */
export const recentsProvider: CommandProvider = {
  id: "app.recents",
  items(): Command[] {
    const current = project.current?.path;
    return project.recents
      .filter((r) => r.path !== current)
      .slice(0, 10)
      .map(
        (r): Command => ({
          id: RECENT_PREFIX + r.path,
          title: "Open {project}",
          params: () => ({ project: r.name }),
          section: "items",
          keywords: [r.path],
          disabled: () => (r.missing ? t("Folder not found") : null),
          run: () => openRecent(r.path, r.name),
        }),
      );
  },
};
