// Control hotkeys: plain decision logic, no DOM or Svelte, so it is unit-testable and cheap.
//
// A layer hotkey toggles one layer of one screen: TAKE if it is off air, CLEAR if it is on air.
// A cue hotkey runs its flow; pressing it again while it runs restarts it (or is ignored: the
// flow's `retrigger`, decided by the server; Stop is the Cues strip's button).
// The keys run through the command registry: Control registers "live.toggle:<screenId>/<layerId>",
// "cue.go:<flowId>" and "live.panic" while it is mounted (./commands.ts) and the shell's keydown
// resolves them through the binding table ($lib/commands/bindings.svelte.ts). Their keys live in
// `UiState.control.hotkeys`: "<screenId>/<layerId>" or "flow/<flowId>" (screen ids are 10
// characters, so "flow" is never one); a leftover "macro/<id>" is treated as "flow/<id>". Combos are
// F1–F12, optionally with Shift ("Shift+F3"), one target per combo. Shift+Escape is PANIC.
// In Studio mode a layer key cues the layer in or out of Preview instead, and Space / Enter take
// the selected screen's Preview (`isStudioTake`).

export {
  FLOW_KEY_PREFIX,
  LEGACY_KEY_PREFIX,
  assignHotkey,
  flowOfKey,
  removeHotkey,
  splitHotkeyKey,
} from "$lib/commands/bindings.svelte";
import { FLOW_KEY_PREFIX, LEGACY_KEY_PREFIX, splitHotkeyKey } from "$lib/commands/bindings.svelte";

/** The parts of a KeyboardEvent the decisions read (a real event fits). */
export interface KeyLike {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  repeat: boolean;
}

export interface Combo {
  shift: boolean;
  /** 1–12. */
  n: number;
}

const COMBO_RE = /^(Shift\+)?F([1-9]|1[0-2])$/;

/** "Shift+F3" → { shift: true, n: 3 }; anything else → null. */
export function parseCombo(s: string): Combo | null {
  const m = COMBO_RE.exec(s);
  return m ? { shift: m[1] !== undefined, n: Number(m[2]) } : null;
}

export function formatCombo(c: Combo): string {
  return `${c.shift ? "Shift+" : ""}F${c.n}`;
}

export function isCombo(s: string): boolean {
  return COMBO_RE.test(s);
}

/** The combo a keydown stands for: F1–F12 with or without Shift, no Ctrl/Alt/Meta. */
export function comboOf(e: KeyLike): string | null {
  if (e.ctrlKey || e.altKey || e.metaKey) return null;
  const m = /^F([1-9]|1[0-2])$/.exec(e.key);
  return m ? formatCombo({ shift: e.shiftKey, n: Number(m[1]) }) : null;
}

/** The `hotkeys` map key of a layer. Layer ids are only unique per screen. */
export function hotkeyKey(screenId: string, layerId: string): string {
  return `${screenId}/${layerId}`;
}

/** The `hotkeys` map key of a cue flow. */
export function flowHotkeyKey(flowId: string): string {
  return FLOW_KEY_PREFIX + flowId;
}

/** A flow's combo, from its "flow/<id>" key or a leftover "macro/<id>" one. */
export function flowHotkey(hotkeys: Readonly<Record<string, string>>, flowId: string): string | null {
  return hotkeys[FLOW_KEY_PREFIX + flowId] ?? hotkeys[LEGACY_KEY_PREFIX + flowId] ?? null;
}

/** Drops a flow's keys (both spellings); the same map when it had none. */
export function dropFlowHotkey(hotkeys: Readonly<Record<string, string>>, flowId: string): Record<string, string> {
  const next = { ...hotkeys };
  delete next[FLOW_KEY_PREFIX + flowId];
  delete next[LEGACY_KEY_PREFIX + flowId];
  return next;
}

/** The key (layer or flow) a combo is mapped to, or null. */
export function ownerKey(hotkeys: Readonly<Record<string, string>>, combo: string): string | null {
  for (const [key, c] of Object.entries(hotkeys)) if (c === combo) return key;
  return null;
}

/** The layer a combo is mapped to, or null (also when a flow owns it). */
export function ownerOf(
  hotkeys: Readonly<Record<string, string>>,
  combo: string,
): { screenId: string; layerId: string } | null {
  const key = ownerKey(hotkeys, combo);
  return key === null ? null : splitHotkeyKey(key);
}

/**
 * Studio mode's TAKE key: Space or Enter, no modifiers, not on repeat, and not `blocked` (typing,
 * or focus on something that uses the key itself: a button, a slider, a dialog…).
 */
export function isStudioTake(e: KeyLike, blocked: boolean): boolean {
  if (e.repeat || blocked || e.shiftKey || e.ctrlKey || e.altKey || e.metaKey) return false;
  return e.key === " " || e.key === "Enter";
}

/** A toggle takes a layer that is off air and clears one that is on air. */
export function toggleDecision(layer: { onAir: boolean }): "take" | "clear" {
  return layer.onAir ? "clear" : "take";
}

export type CaptureAction = { kind: "set"; combo: string } | { kind: "cancel" };

/** While "Set hotkey" waits for a key: an F-key sets it, Esc cancels, anything else is ignored. */
export function captureAction(e: KeyLike): CaptureAction | null {
  if (e.repeat) return null;
  if (e.key === "Escape" && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) return { kind: "cancel" };
  const combo = comboOf(e);
  return combo ? { kind: "set", combo } : null;
}
