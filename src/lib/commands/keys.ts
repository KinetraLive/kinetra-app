// Keyboard helpers shared by the shell, the command registry and pages: "is the user typing?",
// the one canonical spelling of a key binding, and the bindings a keydown stands for.
// Dependency-free: it is part of the app shell.
//
// Canonical spelling (model.rs `is_key_combo` accepts exactly this): modifiers Ctrl, Alt, Shift (each
// at most once, in that order), then one key: A–Z, 0–9, F1–F24, one of ; ' , . / \ [ ] - = ` or a
// named key (Escape, Enter, Space, Tab, Backspace, Delete, Insert, Home, End, PageUp, PageDown,
// ArrowUp/Down/Left/Right). E.g. "Ctrl+Shift+P", "Shift+F3", "Escape". Cmd counts as Ctrl.

/** True when keys typed at `target` are text input (fields, textareas, selects, contenteditable). */
export function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) {
    return !["button", "checkbox", "radio", "range", "submit", "reset", "color", "file"].includes(
      target.type,
    );
  }
  return false;
}

const MODS = ["Ctrl", "Alt", "Shift"] as const;
const MOD_ALIASES: Record<string, (typeof MODS)[number]> = {
  ctrl: "Ctrl",
  control: "Ctrl",
  cmd: "Ctrl",
  meta: "Ctrl",
  alt: "Alt",
  option: "Alt",
  shift: "Shift",
};
const NAMED = [
  "Escape",
  "Enter",
  "Space",
  "Tab",
  "Backspace",
  "Delete",
  "Insert",
  "Home",
  "End",
  "PageUp",
  "PageDown",
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
];
const NAMED_BY_LOWER = new Map(NAMED.map((n) => [n.toLowerCase(), n]));
const KEY_ALIASES: Record<string, string> = {
  esc: "Escape",
  return: "Enter",
  " ": "Space",
  spacebar: "Space",
  del: "Delete",
  ins: "Insert",
  pgup: "PageUp",
  pgdn: "PageDown",
  up: "ArrowUp",
  down: "ArrowDown",
  left: "ArrowLeft",
  right: "ArrowRight",
};
const PUNCTUATION = ";',./\\[]-=`";

/** A key's canonical name ("g" → "G", "esc" → "Escape", "f3" → "F3"), or null. */
function keyName(k: string): string | null {
  if (k.length === 1) {
    if (/[a-z0-9]/i.test(k)) return k.toUpperCase();
    if (PUNCTUATION.includes(k)) return k;
    return k === " " ? "Space" : null;
  }
  const f = /^f([1-9]|1[0-9]|2[0-4])$/i.exec(k);
  if (f) return `F${f[1]}`;
  const lower = k.toLowerCase();
  return NAMED_BY_LOWER.get(lower) ?? KEY_ALIASES[lower] ?? null;
}

/** "ctrl+shift+p" → "Ctrl+Shift+P", "Shift+Esc" → "Shift+Escape"; null when it isn't a key binding. */
export function normalizeKeys(combo: string): string | null {
  const parts = combo.trim().split("+");
  const key = parts.pop();
  if (key === undefined) return null;
  const mods = new Set<string>();
  for (const p of parts) {
    const m = MOD_ALIASES[p.trim().toLowerCase()];
    if (!m || mods.has(m)) return null;
    mods.add(m);
  }
  const name = keyName(key.trim() === "" && key.length > 0 ? " " : key.trim());
  if (!name) return null;
  return [...MODS.filter((m) => mods.has(m)), name].join("+");
}

/** True when `combo` is a key binding in canonical spelling. */
export const isCanonicalKeys = (combo: string): boolean => normalizeKeys(combo) === combo;

/** True when the binding has Ctrl or Alt (such keys may work while typing). */
export const hasCommandModifier = (combo: string): boolean => /^(Ctrl|Alt)\+/.test(combo);

/** The parts of a KeyboardEvent the matching reads (a real event fits). */
export interface KeyLike {
  key: string;
  code?: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

/**
 * The canonical bindings a keydown stands for, best first; empty for a lone modifier. Letters and
 * digits come from the physical key (`code`), so Shift and keyboard layouts don't change them;
 * other keys from the character typed. Punctuation pressed with Shift also stands for the binding
 * without Shift (";" needs Shift on many layouts, Norwegian: Shift+,).
 */
export function eventKeys(e: KeyLike): string[] {
  const code = e.code ?? "";
  let name: string | null;
  if (/^Key[A-Z]$/.test(code)) name = code.slice(3);
  else if (/^Digit[0-9]$/.test(code)) name = code.slice(5);
  else name = keyName(e.key);
  if (!name) return [];
  const ctrl = e.ctrlKey || e.metaKey;
  const head = `${ctrl ? "Ctrl+" : ""}${e.altKey ? "Alt+" : ""}`;
  const primary = `${head}${e.shiftKey ? "Shift+" : ""}${name}`;
  return e.shiftKey && PUNCTUATION.includes(name) ? [primary, `${head}${name}`] : [primary];
}

/** True when `e` is the shortcut `combo` (any spelling `normalizeKeys` accepts). */
export function matchesShortcut(e: KeyLike, combo: string): boolean {
  const c = normalizeKeys(combo);
  return c !== null && eventKeys(e).includes(c);
}

/** A binding as aria-keyshortcuts spells it ("Ctrl+G" → "Control+G"). */
export const ariaKeys = (s: string): string => s.replace(/Ctrl/g, "Control");

/** A binding shortened for tight labels ("Shift+Escape" → "Shift+Esc"). */
export const shortKeys = (s: string): string => s.replace(/Escape$/, "Esc");
