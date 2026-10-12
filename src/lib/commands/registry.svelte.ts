// The app-wide command registry: one list behind the command palette, keyboard shortcuts and menu
// actions. Part of the shell, so it stays small; the palette UI, search and providers are lazy.
//
// Features register their commands for as long as they apply and unregister on teardown:
//   onMount(() => registerCommands([{ id, title, section, run, defaultKeys, keyScope, when }]))
// Keys come from the binding table (./bindings.svelte.ts), never from registration order.
// Pages report what they know with `provideContext`; the shell sets `route` and `project`.
// Titles are messages: shown as `t(title, params?.())`.

import { untrack } from "svelte";
import type House from "lucide-svelte/icons/house";
import { openPreferences, toAppError } from "$lib/api";
import { t, type Msg } from "$lib/i18n";
import { toasts } from "$lib/stores/toasts.svelte";
// Through the project store: both stores stay in the one chunk the picker loads.
import { uiState } from "$lib/stores/project.svelte";
import { PALETTE_RECENT_CAP } from "$lib/types";
import { commandsForKeys } from "./bindings.svelte";
import { eventKeys, hasCommandModifier, isTyping } from "./keys";

export type { Msg };
/** A lucide icon (per-icon import); lucide-svelte's icons are typed as classes, not `Component`. */
export type CommandIcon = typeof House;

// ---- Context -------------------------------------------------------------------------------------

/** Route families. Ids such as "nav.flows" stay when routes are renamed later. */
export type RouteName =
  | "picker" | "dashboard" | "templates" | "templateEditor" | "screens" | "screenEditor"
  | "sources" | "flows" | "flowEditor" | "control" | "settings";

/** Where the user is and what is in focus; drives `when`, `disabled`, ranking and key scopes. */
export interface CommandContext {
  route: RouteName;
  project: boolean;
  /** The screen in focus (Control's selected screen, the layout editor's screen). */
  screenId: string | null;
  /** Layers selected in the layout editor, or the expanded one in Control. */
  layerIds: readonly string[];
  /** Studio mode is on (takes go to Preview). */
  studio: boolean;
}

/** The route family of a pathname ("/templates/abc" → "templateEditor"). */
export function routeOf(pathname: string): RouteName {
  const [, a = "", b] = pathname.split("/");
  switch (a) {
    case "":
      return "picker";
    case "templates":
      return b ? "templateEditor" : "templates";
    case "screens":
      return b ? "screenEditor" : "screens";
    case "flows":
      return b ? "flowEditor" : "flows";
    case "sources":
    case "control":
    case "settings":
    case "dashboard":
      return a;
    default:
      return "dashboard";
  }
}

// ---- Commands ------------------------------------------------------------------------------------

/** Palette sections, in display order. `recent` is filled from history, never assigned. `edit`:
 * editor actions (layout editor, flow editor), listed only where those are registered. */
export type Section = "recent" | "live" | "goto" | "create" | "edit" | "items" | "settings";
/** Prefix filters: ">" commands, "@" screens, "#" templates, "/" settings. */
export type Scope = "command" | "screen" | "template" | "setting";
/** Where a key binding is active; "global" overlaps every scope. */
export type KeyScope = "global" | RouteName;

/** A follow-up step: pick one value from a sub-list, or enter text. */
export type FollowUp =
  | {
      kind: "pick";
      placeholder: Msg;
      /** The choices (already translated titles); searched with the same scorer. */
      options(ctx: CommandContext): readonly { id: string; title: string; keywords?: readonly string[]; detail?: string }[];
      run(optionId: string, ctx: CommandContext): void | Promise<void>;
    }
  | {
      kind: "text";
      placeholder: Msg;
      /** Initial text. */
      value?(ctx: CommandContext): string;
      /** A reason the text can't be used (shown under the input), or null. */
      validate?(text: string): string | null;
      run(text: string, ctx: CommandContext): void | Promise<void>;
    };

export interface Command {
  /** Stable across sessions (Recent and bindings store ids): "nav.control", "cue.go:<flowId>". */
  id: string;
  /** Message key; shown as `t(title, params?.())`. Ends in "…" when it has `next`. */
  title: Msg;
  params?: () => Readonly<Record<string, string | number>>;
  section: Exclude<Section, "recent">;
  /** Default "command"; item providers use "screen" / "template", settings "setting". */
  scope?: Scope;
  /** Extra words the search matches but doesn't show (already translated). */
  keywords?: readonly string[];
  /** lucide icon, per-icon import. */
  icon?: CommandIcon;
  /** Default keys in canonical spelling ("Ctrl+G", "F1", "Shift+Escape"); bindings override them. */
  defaultKeys?: readonly string[];
  /** Where its keys work. Default "global". */
  keyScope?: KeyScope;
  /** Listed and runnable only when true. Default: always. */
  when?: (ctx: CommandContext) => boolean;
  /** Listed but disabled, with the returned reason as its tooltip. */
  disabled?: (ctx: CommandContext) => string | null;
  /** Added to the search score in this context (live actions in Control rank first). */
  rank?: (ctx: CommandContext) => number;
  /** A live action (Take, Clear, GO, Update, Panic, Next): repeats are ignored and its keys never
   * fire while typing (unless `whileTyping`). */
  live?: boolean;
  /** Fires on key repeat (e.g. nudges). Never with `live`. */
  repeatable?: boolean;
  /** Second Enter in the palette ("Press Enter again to confirm"). Keys run at once. */
  confirm?: boolean;
  /** Not recorded in Recent. */
  noRecent?: boolean;
  /** Not listed in the palette (its keys still work and Keyboard shortcuts lists it): e.g. a
   * layer's Take / Clear key, whose palette rows are the Control provider's explicit items. */
  hidden?: boolean;
  /** Plain-key bindings (no Ctrl/Alt) never fire while typing; this also blocks Ctrl/Alt ones. */
  notWhileTyping?: boolean;
  /** Its keys fire even while typing, plain keys and `live` included (PANIC, the palette, Ctrl+,). */
  whileTyping?: boolean;
  /** Either `run`, or `next` for a follow-up step (the palette shows the sub-list / text input). */
  run?: (ctx: CommandContext) => void | Promise<void>;
  next?: FollowUp;
}

/** Entries from project data, computed only while the palette is open. Synchronous. */
export interface CommandProvider {
  id: string;
  items(ctx: CommandContext): readonly Command[];
}

const DEFAULT_CONTEXT: CommandContext = {
  route: "dashboard",
  project: false,
  screenId: null,
  layerIds: [],
  studio: false,
};

class Registry {
  /** In registration order; replaced wholesale. */
  list = $state.raw<readonly Command[]>([]);
  providers = $state.raw<readonly CommandProvider[]>([]);
  patches = $state.raw<readonly Partial<CommandContext>[]>([]);
  context: CommandContext = $derived.by(() => {
    const c: Record<string, unknown> = { ...DEFAULT_CONTEXT };
    for (const p of this.patches) for (const [k, v] of Object.entries(p)) if (v !== undefined) c[k] = v;
    return c as unknown as CommandContext;
  });

  // Mutations read the current lists untracked: they are called from effects (a page registering
  // while mounted) and must not make those effects depend on the registry.
  register(cmds: readonly Command[]): () => void {
    const ids = new Set(cmds.map((c) => c.id));
    untrack(() => (this.list = [...this.list.filter((c) => !ids.has(c.id)), ...cmds]));
    return () =>
      untrack(() => {
        if (cmds.some((c) => this.list.includes(c))) this.list = this.list.filter((c) => !cmds.includes(c));
      });
  }
}

export const registry = new Registry();

/** Adds commands (replacing any with the same id); returns the function that removes them. */
export const registerCommands = (cmds: readonly Command[]): (() => void) => registry.register(cmds);

/** Adds a provider of project items (asked only while the palette is open). */
export function registerProvider(p: CommandProvider): () => void {
  untrack(() => (registry.providers = [...registry.providers.filter((x) => x.id !== p.id), p]));
  return () =>
    untrack(() => {
      if (registry.providers.includes(p)) registry.providers = registry.providers.filter((x) => x !== p);
    });
}

/** Pages report what they know (the shell: `route`, `project`). Returns the function that drops it. */
export function provideContext(patch: Partial<CommandContext>): () => void {
  untrack(() => (registry.patches = [...registry.patches, patch]));
  return () => untrack(() => (registry.patches = registry.patches.filter((p) => p !== patch)));
}

export const currentContext = (): CommandContext => registry.context;

/** Every registered command (reactive; registration order). */
export const commands = (): readonly Command[] => registry.list;

/** The providers' items in this context (the palette calls it while open). */
export function providerItems(ctx: CommandContext = currentContext()): Command[] {
  return registry.providers.flatMap((p) => p.items(ctx));
}

/** The registered command with this id (allowed or not), else a provider's item with it. */
export function getCommand(id: string): Command | undefined {
  return registry.list.find((c) => c.id === id) ?? providerItems().find((c) => c.id === id);
}

/** The translated title of a command. */
export const titleOf = (c: Command): string => t(c.title, c.params?.());

/** Listed and runnable in `ctx`: `when` allows it and it isn't disabled. */
export const isAllowed = (c: Command, ctx: CommandContext = currentContext()): boolean =>
  (c.when?.(ctx) ?? true) && !c.disabled?.(ctx);

// ---- Running -------------------------------------------------------------------------------------

function failed(e: unknown) {
  toasts.push({ kind: "error", message: toAppError(e).message });
}

async function execute(f: () => void | Promise<void>): Promise<true> {
  try {
    await f();
  } catch (e) {
    failed(e);
  }
  return true;
}

/** The last commands run from the palette, newest first, for the open project; ids that are
 * neither registered nor a provider's item right now are skipped. */
export function recentIds(): readonly string[] {
  const ids = uiState.value?.palette?.recent ?? [];
  if (ids.length === 0) return ids;
  const known = new Set(registry.list.map((c) => c.id));
  let items: Set<string> | null = null;
  return ids.filter((id) => known.has(id) || (items ??= new Set(providerItems().map((c) => c.id))).has(id));
}

function remember(id: string) {
  const v = uiState.value;
  if (!v) return;
  const before = v.palette?.recent ?? [];
  const recent = [id, ...before.filter((x) => x !== id)].slice(0, PALETTE_RECENT_CAP);
  if (recent.length === before.length && recent.every((x, i) => x === before[i])) return;
  uiState.update({ ...v, palette: { recent } }).catch(() => {});
}

/**
 * Runs a command by id when it is known, allowed (`when`) and not disabled; records Recent unless
 * `noRecent`; errors become a toast. Commands with `next` need `runFollowUp`. True when it ran.
 */
export async function runCommand(id: string): Promise<boolean> {
  const c = getCommand(id);
  const ctx = currentContext();
  if (!c?.run || !isAllowed(c, ctx)) return false;
  const run = c.run;
  if (!c.noRecent) remember(c.id);
  return execute(() => run(ctx));
}

/** Runs a command's follow-up step with the picked option id or the entered text. */
export async function runFollowUp(id: string, value: string): Promise<boolean> {
  const c = getCommand(id);
  const ctx = currentContext();
  const next = c?.next;
  if (!c || !next || !isAllowed(c, ctx)) return false;
  if (next.kind === "text" && next.validate?.(value)) return false;
  if (!c.noRecent) remember(c.id);
  return execute(() => next.run(value, ctx));
}

let followUpOpener: ((id: string) => void) | null = null;
/** The shell's way to show a command's follow-up step when its key is pressed (the palette). */
export function setFollowUpOpener(open: ((id: string) => void) | null): void {
  followUpOpener = open;
}

let paused = 0;
/** No key runs a command until the returned resume is called (Keyboard shortcuts records a key). */
export function pauseShortcuts(): () => void {
  paused++;
  let resumed = false;
  return () => {
    if (!resumed) {
      resumed = true;
      paused--;
    }
  };
}

const scopeOk = (c: Command, ctx: CommandContext) => !c.keyScope || c.keyScope === "global" || c.keyScope === ctx.route;

function typingOk(c: Command, keys: string, typing: boolean): boolean {
  if (!typing || c.whileTyping) return true;
  return !c.live && !c.notWhileTyping && hasCommandModifier(keys);
}

function conflict(keys: string, cmds: readonly Command[]) {
  toasts.push({
    kind: "error",
    message: t("{keys} is assigned to {commands}. Nothing ran.", { keys, commands: cmds.map(titleOf).join(", ") }),
    timeoutMs: 6000,
    // Opens the Preferences window at Keyboard shortcuts.
    action: { label: t("Keyboard shortcuts"), run: () => void openPreferences("shortcuts").catch(() => {}) },
  });
}

/**
 * The shell's keydown (capture phase): resolves `e` through the binding table to the commands
 * allowed here (key scope, `when`, typing rule). Exactly one runs (true; the caller prevents the
 * default); two or more run nothing and toast the conflict (true); none → false. Repeats are
 * swallowed (true) unless the command is `repeatable`. A disabled command leaves the key alone.
 * Nothing runs while paused (`pauseShortcuts`).
 */
export function runShortcut(e: KeyboardEvent): boolean {
  if (paused > 0) return false;
  const ctx = currentContext();
  const typing = isTyping(e.target);
  for (const keys of eventKeys(e)) {
    const hits = commandsForKeys(keys).filter(
      (c) => scopeOk(c, ctx) && (c.when?.(ctx) ?? true) && typingOk(c, keys, typing),
    );
    if (hits.length === 0) continue;
    if (e.repeat) {
      const c = hits.length === 1 ? hits[0] : null;
      if (!c?.repeatable || c.live || c.disabled?.(ctx)) return true;
    } else if (hits.length > 1) {
      conflict(keys, hits);
      return true;
    }
    const c = hits[0];
    if (c.disabled?.(ctx)) return false;
    if (c.run) {
      const run = c.run;
      void execute(() => run(ctx));
      return true;
    }
    if (c.next && followUpOpener) {
      followUpOpener(c.id);
      return true;
    }
    return false;
  }
  return false;
}
