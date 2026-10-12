// The lazy part of the layer / cue context menu (./menu.ts and Control's ./eager.svelte.ts open it):
// one Popup mounted on document.body on first use, showing either the menu or the Assign MIDI
// popover at a point. The mappings are watched while it is open (for "Remove MIDI" and conflicts).
// Closing returns focus to the row it was opened from.

import { mount } from "svelte";
import type { ControlCtl } from "$lib/components/control/control.svelte";
import { cueText } from "$lib/components/flows/cue";
import { controls } from "./controls.svelte";
import type { MenuSpec } from "./menu";
import Popup from "./Popup.svelte";

export const popup = $state<{ spec: MenuSpec | null; mode: "menu" | "assign"; x: number; y: number }>({
  spec: null,
  mode: "menu",
  x: 0,
  y: 0,
});

let mounted = false;
let anchor: HTMLElement | null = null;
let unwatch: (() => void) | null = null;

/**
 * Opens the menu for `spec`, at the pointer (`at`) or, from the keyboard, under the focused
 * element inside `from` (else `from`).
 */
export function openMenu(spec: MenuSpec, from: HTMLElement, at: { x: number; y: number } | null): void {
  const focus = document.activeElement;
  anchor = focus instanceof HTMLElement && from.contains(focus) ? focus : from;
  const r = anchor.getBoundingClientRect();
  unwatch ??= controls.watch();
  popup.spec = spec;
  popup.mode = "menu";
  popup.x = at ? at.x : r.left + 8;
  popup.y = at ? at.y : r.bottom;
  if (!mounted) {
    mount(Popup, { target: document.body });
    mounted = true;
  }
}

/** Control's side panel: the menu of a layer row (`data-layer`, on the selected screen) or a cue
 * (`data-cue`), with "Set hotkey" through Control (a layer's row expands to show the capture). */
export function openControlMenu(ctl: ControlCtl, row: HTMLElement, at: { x: number; y: number } | null): void {
  const layerId = row.dataset.layer;
  const screenId = ctl.selectedId;
  if (layerId && screenId) {
    const key = `${screenId}/${layerId}`;
    // Through ctl, not the live store: importing a module of the route from here would split it out.
    const name = ctl.selected?.layers.find((l) => l.id === layerId)?.name ?? layerId;
    openMenu(
      {
        target: { kind: "layer", screenId, layerId },
        name,
        setHotkey: () => {
          if (!ctl.expanded.has(key)) ctl.toggleExpanded(key);
          if (ctl.capturing !== key) ctl.startCapture(key);
        },
      },
      row,
      at,
    );
    return;
  }
  const flowId = row.dataset.cue;
  if (!flowId) return;
  const flow = ctl.flows.cues.find((f) => f.id === flowId);
  const key = `flow/${flowId}`;
  openMenu(
    {
      target: { kind: "cue", flowId },
      name: flow ? (cueText(flow) ?? flow.name) : flowId,
      setHotkey: () => {
        if (ctl.capturing !== key) ctl.startCapture(key);
      },
    },
    row,
    at,
  );
}

/** The menu's "Assign MIDI…": the popover in the menu's place. */
export const showAssign = (): void => {
  popup.mode = "assign";
};

export function closePopup(refocus = true): void {
  if (!popup.spec) return;
  popup.spec = null;
  unwatch?.();
  unwatch = null;
  const a = anchor;
  anchor = null;
  if (refocus && a?.isConnected) a.focus();
}
