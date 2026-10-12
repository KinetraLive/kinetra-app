// The native menu as data: which registry commands go where, and `buildMenuSpec`, which turns that
// layout plus the current registry, bindings and context into the `MenuSpec` sent to Rust
// (`set_app_menu`). Pure: everything it reads comes in through `MenuInput`, so it is unit-tested
// without the registry, Tauri or Svelte.
//
// Rules:
//   - An item is a registry command id (the `menu` event sends it back; ./sync runs it).
//   - Text: the item's own menu text (short: "Take", "Sidebar") or the command's title, through
//     `t()`; "&" in it is doubled (a literal ampersand, never a mnemonic). Menu titles carry their
//     mnemonic ("&File").
//   - Keys: the command's first effective binding (the user's, else its default); some items show
//     another command's keys (Preferences… shows "Go to Preferences"'s Ctrl+,) or fixed text
//     (Undo / Redo: the editors handle Ctrl+Z / Ctrl+Y themselves).
//   - Enabled: registered (or a fallback exists) && `when` && !`disabled`.
//   - Separators never lead, trail or repeat; a submenu without items is disabled.
//   - Windows: Undo / Redo are registry items (`edit.undo` / `edit.redo`); macOS: the predefined
//     items (WKWebView needs them). Cut / Copy / Paste / Select all are predefined everywhere.
//     The macOS app menu (About, Preferences…, Quit) is built by Rust from `help.about`,
//     `app.preferences` and `app.exit`, which it leaves out of File / Help there.

import type { Command, CommandContext } from "../registry.svelte";
import { t, type Msg } from "$lib/i18n";
import type { MenuItem, MenuNode, MenuPredefined, MenuSpec, MenuSubmenu, UiMode } from "$lib/types";

export interface MenuInput {
  ctx: CommandContext;
  mac: boolean;
  /** A registered command (or a provider item) by id; else the menu's fallback for it. */
  command(id: string): Command | undefined;
  /** The effective keys of a command id (bindings over defaults). */
  keys(id: string): readonly string[];
  /** Cue commands (`cue.go:<flowId>`) in Control's order. */
  cues: readonly Command[];
  /** "Open recent" items (`app.openRecent:<path>`), newest first. */
  recents: readonly Command[];
  /** Output: one checkable item per configured output (`output.toggle:<id>`), its menu text
   * ("Output 1 — DELL U2720Q") and whether it is on. */
  outputs: readonly { id: string; text: string; checked: boolean }[];
  uiMode: UiMode;
  sidebarPinned: boolean;
}

interface ItemOpts {
  /** Menu text instead of the command's title. */
  text?: Msg;
  /** Show this command's keys instead of the item's own. */
  keysOf?: string;
  /** Fixed key text (never a binding). */
  keys?: string;
  checked?: boolean;
}

/** Undo / Redo keys on Windows (shown only: the focused editor handles them). */
export const UNDO_KEYS = "Ctrl+Z";
export const REDO_KEYS = "Ctrl+Y";

/** A literal "&" in menu text (Windows reads a single one as a mnemonic). */
export const escapeMenuText = (s: string): string => s.replace(/&/g, "&&");

const allowed = (c: Command, ctx: CommandContext) => (c.when?.(ctx) ?? true) && !c.disabled?.(ctx);

function item(input: MenuInput, id: string, o: ItemOpts = {}): MenuItem {
  const c = input.command(id);
  const text = o.text !== undefined ? t(o.text) : c ? t(c.title, c.params?.()) : id;
  const keys = o.keys ?? input.keys(o.keysOf ?? id)[0] ?? null;
  const out: MenuItem = { kind: "item", id, text: escapeMenuText(text), keys, enabled: !!c && allowed(c, input.ctx) };
  if (o.checked !== undefined) out.checked = o.checked;
  return out;
}

const SEP: MenuNode = { kind: "separator" };
const predefined = (p: MenuPredefined["item"]): MenuPredefined => ({ kind: "predefined", item: p });

/** Drops leading, trailing and repeated separators. */
function tidy(nodes: MenuNode[]): MenuNode[] {
  const out: MenuNode[] = [];
  for (const n of nodes) {
    if (n.kind === "separator" && (out.length === 0 || out[out.length - 1].kind === "separator")) continue;
    out.push(n);
  }
  while (out.length > 0 && out[out.length - 1].kind === "separator") out.pop();
  return out;
}

function submenu(id: string, text: string, nodes: MenuNode[], enabled = true): MenuSubmenu {
  const items = tidy(nodes);
  return { kind: "submenu", id, text, enabled: enabled && items.length > 0, items };
}

/** Builds the whole menu bar. */
export function buildMenuSpec(input: MenuInput): MenuSpec {
  const { mac } = input;
  const recents = input.recents.map((c) => item(input, c.id));
  const file = submenu("menu.file", t("&File"), [
    item(input, "app.newProject", { text: "New project…" }),
    item(input, "app.openProject", { text: "Open project…" }),
    submenu("menu.file.recent", t("Open recent"), [...recents, SEP, item(input, "app.clearRecents", { text: "Clear recent" })], recents.length > 0),
    SEP,
    submenu("menu.file.import", t("Import"), [item(input, "app.importSettings", { text: "Settings file…" })]),
    submenu("menu.file.export", t("Export"), [item(input, "app.exportSettings", { text: "Settings file…" })]),
    SEP,
    item(input, "app.preferences", { text: "Preferences…", keysOf: "nav.settings" }),
    SEP,
    item(input, "app.exit", { text: "Exit" }),
  ]);

  const edit = submenu("menu.edit", t("&Edit"), [
    ...(mac
      ? [predefined("undo"), predefined("redo")]
      : [item(input, "edit.undo", { text: "Undo", keys: UNDO_KEYS }), item(input, "edit.redo", { text: "Redo", keys: REDO_KEYS })]),
    SEP,
    predefined("cut"),
    predefined("copy"),
    predefined("paste"),
    SEP,
    predefined("selectAll"),
  ]);

  const view = submenu("menu.view", t("&View"), [
    item(input, "view.simple", { text: "Simple mode", checked: input.uiMode === "simple" }),
    item(input, "view.advanced", { text: "Advanced mode", checked: input.uiMode === "advanced" }),
    SEP,
    submenu("menu.view.panels", t("Panels"), [item(input, "shell.pin", { text: "Sidebar", checked: input.sidebarPinned })]),
    SEP,
    item(input, "view.zoomIn", { text: "Zoom in" }),
    item(input, "view.zoomOut", { text: "Zoom out" }),
    item(input, "view.actualSize", { text: "Actual size" }),
    SEP,
    item(input, "view.fullscreen", { text: "Full screen" }),
  ]);

  const cues = input.cues.map((c) => item(input, c.id));
  const show = submenu("menu.show", t("&Show"), [
    item(input, "show.take", { text: "Take" }),
    item(input, "show.clear", { text: "Clear" }),
    SEP,
    item(input, "live.clearAll", { text: "Clear all" }),
    item(input, "live.panic", { text: "Panic" }),
    SEP,
    // Without cues (outside Control) a disabled, empty "Go" keeps the menu's shape.
    submenu("menu.show.go", t("Go"), cues),
  ]);

  // Displays… always; the outputs and All outputs off once one exists.
  const output = submenu("menu.output", t("&Output"), [
    item(input, "output.displays", { text: "Displays…" }),
    ...(input.outputs.length > 0
      ? [SEP, ...input.outputs.map((o) => item(input, o.id, { text: o.text, checked: o.checked })), SEP, item(input, "output.allOff", { text: "All outputs off" })]
      : []),
  ]);

  const help = submenu("menu.help", t("&Help"), [
    item(input, "help.docs", { text: "Documentation" }),
    item(input, "help.shortcuts", { text: "Keyboard shortcuts" }),
    item(input, "help.updates", { text: "Check for updates…" }),
    SEP,
    item(input, "help.about", { text: "About Kinetra" }),
  ]);

  return { menus: [file, edit, view, show, output, help] };
}
