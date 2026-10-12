// The right-click menu of a layer (the layout editor's Layers panel; Control's side panel uses the
// delegated `sideMenu` in ./eager.svelte.ts): "Assign MIDI…", "Remove MIDI" when assigned, and
// "Set hotkey" when the spec offers it. This module is the small part the panel imports; the menu,
// the Assign MIDI popover and the mappings load on the first open (./host.svelte.ts).
//
//   <li oncontextmenu={(e) => contextMenu(e, spec)} onkeydown={(e) => contextMenu(e, spec)}>
//
// Keyboard: Shift+F10 and the context-menu key open it too (on the focused row).

export type MenuTarget = { kind: "layer"; screenId: string; layerId: string } | { kind: "cue"; flowId: string };

export interface MenuSpec {
  target: MenuTarget;
  /** The layer's or cue's name ("Assign MIDI to Guest"). */
  name: string;
  /** "Set hotkey" (absent: not offered). */
  setHotkey?: () => void;
}

/** Shift+F10 or the context-menu key. */
export const isMenuKey = (e: KeyboardEvent): boolean =>
  e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey);

/** Opens the menu for a `contextmenu` event or a menu key (other keys are ignored). */
export function contextMenu(e: MouseEvent | KeyboardEvent, spec: MenuSpec): void {
  if (e.type === "keydown" && !isMenuKey(e as KeyboardEvent)) return;
  e.preventDefault();
  const m = e as MouseEvent;
  const at = e.type === "contextmenu" && (m.clientX || m.clientY) ? { x: m.clientX, y: m.clientY } : null;
  const from = e.currentTarget as HTMLElement;
  void import("./lazy")
    .then((l) => l.host())
    .then((h) => h.openMenu(spec, from, at));
}
