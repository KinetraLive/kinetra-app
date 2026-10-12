// The layout editor's commands for the command palette and their shortcuts. ScreenEditor registers
// them while it is mounted (so they exist, and their keys work, only in the editor: key scope
// "screenEditor", never while typing; palette section "Edit"); the toolbar shows their keys (after
// the user's bindings, `keyHint`) in its tooltips. Also "Rename screen…" (a text step validated
// like the name field) and the provider "editor.layers": "Edit {layer}" selects that layer.

import { keysFor } from "$lib/commands/bindings.svelte";
import { ariaKeys as toAria } from "$lib/commands/keys";
import type { Command, CommandContext, CommandProvider } from "$lib/commands/registry.svelte";
import { t } from "$lib/i18n";
import { screenNameError } from "./model";
import type { CropOverlay, ScreenEditorView } from "$lib/types";
import { SCREEN_CROP_OVERLAYS } from "./shared";
// "Switch <layer> to <variant>" (registered by ScreenEditor while a layer has several variants).
export { docVariantLayers, watchVariantCommands } from "../variants.svelte";
// Any style change (palette, gallery) saves the editor first and reloads it after.
export { styleHook } from "$lib/components/styles/styles";
import type { AlignKind } from "./align";

export const SHORTCUTS = {
  snap: "Ctrl+;",
  guides: "Ctrl+'",
  grid: "Ctrl+G",
  safe: "Ctrl+Shift+S",
  crop: "Ctrl+Shift+O",
  selectAll: "Ctrl+A",
  left: "Alt+A",
  center: "Alt+H",
  right: "Alt+D",
  top: "Alt+W",
  middle: "Alt+V",
  bottom: "Alt+S",
  distributeX: "Alt+Shift+H",
  distributeY: "Alt+Shift+V",
} as const;

export const ALIGN_TITLES: Record<AlignKind, string> = {
  left: "Align left",
  center: "Align center",
  right: "Align right",
  top: "Align top",
  middle: "Align middle",
  bottom: "Align bottom",
};

/** The crop overlay after `c` (none → 9:16 → 4:5 → 1:1 → 4:3 → none). */
export function nextCrop(c: CropOverlay): CropOverlay {
  const i = SCREEN_CROP_OVERLAYS.indexOf(c);
  return SCREEN_CROP_OVERLAYS[(i + 1) % SCREEN_CROP_OVERLAYS.length];
}

export interface EditorActions {
  view: () => ScreenEditorView;
  setView: (patch: Partial<ScreenEditorView>) => void;
  selectionSize: () => number;
  layerCount: () => number;
  hasGuides: () => boolean;
  selectAll: () => void;
  align: (kind: AlignKind) => void;
  distribute: (axis: "x" | "y") => void;
  clearGuides: () => void;
  toggleLock: () => void;
  editSafeAreas: () => void;
}

/** What every layout editor command shares: its section, key scope and typing rule. */
const base = { section: "edit", keyScope: "screenEditor", notWhileTyping: true } as const;

/** The command id of each toolbar shortcut (`SHORTCUTS` holds their default keys). */
export const SHORTCUT_COMMANDS: Record<keyof typeof SHORTCUTS, string> = {
  snap: "editor.snap",
  guides: "editor.guides",
  grid: "editor.grid",
  safe: "editor.safe",
  crop: "editor.crop",
  selectAll: "editor.selectAll",
  left: "editor.align.left",
  center: "editor.align.center",
  right: "editor.align.right",
  top: "editor.align.top",
  middle: "editor.align.middle",
  bottom: "editor.align.bottom",
  distributeX: "editor.distribute.x",
  distributeY: "editor.distribute.y",
};

/** A toolbar shortcut's keys after the user's bindings: `label` ("Grid (Ctrl+G)", or just
 * "Grid" when unbound) and `aria` (aria-keyshortcuts, or undefined). */
export function keyHint(name: keyof typeof SHORTCUTS, label: string): { label: string; aria: string | undefined } {
  const keys = keysFor(SHORTCUT_COMMANDS[name]);
  return keys.length > 0 ? { label: `${label} (${keys[0]})`, aria: keys.map(toAria).join(" ") } : { label, aria: undefined };
}

export interface EditorItems {
  /** The screen's layers (front first is fine; any order). */
  layers: () => readonly { id: string; name: string }[];
  select: (layerId: string) => void;
  /** The screen's name now. */
  name: () => string;
  rename: (name: string) => void | Promise<void>;
}

/** "Edit {layer}" for every layer of the open screen (palette only, in the layout editor). */
export function editorProvider(a: EditorItems): CommandProvider {
  return {
    id: "editor.layers",
    items: (ctx: CommandContext) =>
      ctx.route !== "screenEditor"
        ? []
        : a.layers().map(
            (l): Command => ({
              id: `editor.layer:${l.id}`,
              title: "Edit {layer}",
              params: () => ({ layer: l.name }),
              section: "items",
              keywords: [t("Layer")],
              run: () => a.select(l.id),
            }),
          ),
  };
}

/** "Rename screen…": a text step with the current name, validated like the name field. */
export function renameCommand(a: EditorItems): Command {
  return {
    id: "screen.rename",
    title: "Rename screen…",
    section: "edit",
    keyScope: "screenEditor",
    next: {
      kind: "text",
      placeholder: "Screen name",
      value: a.name,
      validate: (s) => {
        const e = screenNameError(s);
        return e ? t(e) : null;
      },
      run: (s) => a.rename(s.trim()),
    },
  };
}
const keys = (k: string | undefined) => (k ? [k] : undefined);

export function editorCommands(a: EditorActions): Command[] {
  const toggle = (id: string, title: string, key: keyof ScreenEditorView, shortcut?: string): Command => ({
    ...base,
    id: `editor.${id}`,
    title,
    defaultKeys: keys(shortcut),
    run: () => a.setView({ [key]: !a.view()[key] }),
  });
  const needs = (ok: () => boolean, reason: string) => () => (ok() ? null : t(reason));
  const kinds: AlignKind[] = ["left", "center", "right", "top", "middle", "bottom"];
  return [
    toggle("snap", "Toggle snapping", "snap", SHORTCUTS.snap),
    toggle("guides", "Toggle rulers and guides", "guides", SHORTCUTS.guides),
    toggle("grid", "Toggle grid", "grid", SHORTCUTS.grid),
    {
      id: "editor.safe",
      title: "Toggle action and title safe",
      ...base,
      defaultKeys: [SHORTCUTS.safe],
      run: () => {
        const v = a.view();
        const on = !(v.actionSafe && v.titleSafe);
        a.setView({ actionSafe: on, titleSafe: on });
      },
    },
    toggle("cross", "Toggle center cross", "centerCross"),
    toggle("customSafe", "Toggle custom safe areas", "customSafe"),
    { ...base, id: "editor.editSafe", title: "Edit custom safe areas…", run: a.editSafeAreas },
    {
      id: "editor.crop",
      title: "Cycle crop overlay",
      ...base,
      defaultKeys: [SHORTCUTS.crop],
      run: () => a.setView({ crop: nextCrop(a.view().crop) }),
    },
    {
      id: "editor.selectAll",
      title: "Select all layers",
      ...base,
      defaultKeys: [SHORTCUTS.selectAll],
      run: a.selectAll,
      disabled: needs(() => a.layerCount() > 0, "No layers yet"),
    },
    ...kinds.map(
      (k): Command => ({
        id: `editor.align.${k}`,
        title: ALIGN_TITLES[k],
        ...base,
        defaultKeys: [SHORTCUTS[k]],
        run: () => a.align(k),
        disabled: needs(() => a.selectionSize() > 0, "Select a layer first"),
      }),
    ),
    {
      id: "editor.distribute.x",
      title: "Distribute horizontally",
      ...base,
      defaultKeys: [SHORTCUTS.distributeX],
      run: () => a.distribute("x"),
      disabled: needs(() => a.selectionSize() >= 3, "Select three or more layers"),
    },
    {
      id: "editor.distribute.y",
      title: "Distribute vertically",
      ...base,
      defaultKeys: [SHORTCUTS.distributeY],
      run: () => a.distribute("y"),
      disabled: needs(() => a.selectionSize() >= 3, "Select three or more layers"),
    },
    {
      id: "editor.lock",
      title: "Lock or unlock aspect ratio",
      ...base,
      run: a.toggleLock,
      disabled: needs(() => a.selectionSize() === 1, "Select one layer"),
    },
    { ...base, id: "editor.clearGuides", title: "Clear guides", run: a.clearGuides, disabled: needs(a.hasGuides, "No guides") },
  ];
}

/** A shortcut as aria-keyshortcuts spells it ("Ctrl+G" → "Control+G"). */
export { ariaKeys } from "$lib/commands/keys";
