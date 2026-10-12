// Control's part of MIDI, in LayerBody's chunk (which Control requests when it loads; the page,
// rows and cues reach it through LayerRow's `lazy.body`, so the route's own chunk only gets the
// property reads). Everything else loads through ./lazy.ts on first use:
//   - `midiChips`: layer key "<screen>/<layer>" or "flow/<id>" → the MIDI sources assigned to it
//     (the "MIDI" chips next to the hotkey badges);
//   - `watchControls()`: loads and follows the project's mappings while Control is mounted;
//   - `sideMenu(e, ctl)`: one delegated handler on Control's side panel: a right-click (or Shift+F10 /
//     the context-menu key) on a layer row (`data-layer`) or cue (`data-cue`) opens the lazy context
//     menu: "Assign MIDI…", "Remove MIDI" when assigned, "Set hotkey".

import type { ControlCtl } from "$lib/components/control/control.svelte";

export const midiChips = $state<{ map: ReadonlyMap<string, readonly string[]> }>({ map: new Map() });

const lazy = () => import("./lazy");

/** Loads and follows the mappings (fills `midiChips`); returns the unwatch (safe before the load). */
export function watchControls(): () => void {
  let off: (() => void) | null = null;
  let stopped = false;
  void lazy()
    .then((l) => l.store())
    .then(
      (m) => {
        if (!stopped) off = m.controls.watch((d) => (midiChips.map = m.chips(d.mappings)));
      },
      () => {},
    );
  return () => {
    stopped = true;
    off?.();
  };
}

/** Control's side panel: the context menu of the layer row or cue the event is on. */
export function sideMenu(e: MouseEvent | KeyboardEvent, ctl: ControlCtl): void {
  const k = e as KeyboardEvent;
  if (e.type === "keydown" && k.key !== "ContextMenu" && !(k.key === "F10" && k.shiftKey)) return;
  const t = e.target as Element;
  // Text fields keep their own menu (copy, paste).
  const row = t.closest?.("input,textarea,select,[contenteditable]") ? null : t.closest?.<HTMLElement>("[data-layer],[data-cue]");
  if (!row) return;
  e.preventDefault();
  const m = e as MouseEvent;
  const at = e.type === "contextmenu" && (m.clientX || m.clientY) ? { x: m.clientX, y: m.clientY } : null;
  void lazy()
    .then((l) => l.host())
    .then((h) => h.openControlMenu(ctl, row, at));
}
