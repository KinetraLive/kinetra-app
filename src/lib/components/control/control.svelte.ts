// Control page state and actions: the operator view's single controller. Components read from it
// and call its methods; the page wires its lifecycle.
//
//   const ctl = new ControlCtl(isTyping);
//   $effect(() => ctl.connect());               // live + server mirrors, events, lists
//   $effect(() => void ctl.loadDoc(ctl.selectedId)); // bindings and template ids of the selection
//
// Live state (on air, pending, field values) comes from the `live` store, kept current by
// `live-changed`. TAKE/CLEAR/Clear/PANIC update it optimistically and roll back on failure (only
// when nothing newer arrived meanwhile). Field edits are debounced per layer (150 ms) into one
// `setLayerFields` call and flushed before a TAKE; the field being edited is never overwritten by
// an incoming update (a local draft wins while focused or unsent). Layout choices write `uiState`.
//
// Automation (all server-side): timer actions go straight to `setTimer` (no debounce; the
// reply and `live-changed` carry the new state). Sources follow `source-changed` (auto-advance
// steps, arming, `nextAt`). Flows: `flows` (FlowsState) holds the list and its run states;
// cue hotkeys are "flow/<id>" in the same `hotkeys` map; deleting a flow drops its hotkey. The
// hotkeys run as commands (./commands.svelte.ts, registered while the page is mounted) through the
// shell's keydown; "Set hotkey" writes them with the binding table's `setKeys`. The
// linear flow editor is a lazy chunk opened through `flowEditor`. Automation off is
// `ui.automationOff`, written through uiState (the backend reacts to the saved value).
//
// Studio mode (Preview / Program): `studio` (./studio/studio.svelte.ts). While it is on, Space /
// Enter take the selected screen's Preview (not while typing or on a control that uses the key)
// and a layer's F-key cues it in or out of Preview instead of taking it.

import { untrack } from "svelte";
import { on, toAppError } from "$lib/api";
import { setAutoAdvance, setTimer } from "$lib/api/automation";
import {
  liveClear,
  liveClearAll,
  liveHide,
  livePanic,
  liveShow,
  setLayerFields,
} from "$lib/api/live";
import { getScreen } from "$lib/api/screens";
import { listSources, stepSource } from "$lib/api/sources";
import { listTemplates } from "$lib/api/templates";
import { live } from "$lib/stores/live.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import {
  TEST_SCREEN_ID,
  type AutoAdvance,
  type ControlViewState,
  type LiveLayerView,
  type LiveScreenView,
  type ScreenDoc,
  type SourceStep,
  type SourceSummary,
  type TemplateScreenLayer,
  type TemplateSummary,
  type TimerAction,
} from "$lib/types";
import { bindingText } from "$lib/components/fields/binding-label";
import { FlowsState } from "$lib/components/flows/flows.svelte";
import { FlowEditorLauncher } from "$lib/components/flows/launcher.svelte";
import { LayerEdits } from "./edits";
import { StudioCtl } from "./studio/studio.svelte";
import { commandOfProjectKey, setKeys } from "$lib/commands/bindings.svelte";
import { captureAction, hotkeyKey, isStudioTake, splitHotkeyKey, toggleDecision } from "./hotkeys";

/** What "Auto" turns on with when a source has no settings yet. */
export const DEFAULT_AUTO: AutoAdvance = { everyMs: 5000, loop: true, transition: "update" };

export const EDIT_DEBOUNCE_MS = 150;
export const GRID_COLUMNS = { min: 1, max: 6 } as const;
/** Most live iframes in the grid (each is a WebView2 renderer process). */
export const CONTROL_LIVE_CAP = 6;

export const DEFAULT_CONTROL: ControlViewState = {
  view: "grid",
  gridColumns: 2,
  selectedScreenId: null,
  hiddenScreenIds: [],
  liveEdit: "onTake",
  hotkeys: {},
  automationOff: false,
};

/** Screens the operator runs: not the dev test screen, not reserved `__` screens. */
export function isUserScreen(id: string): boolean {
  return id !== TEST_SCREEN_ID && !id.startsWith("__");
}

export function onAirCount(view: LiveScreenView): number {
  let n = 0;
  for (const l of view.layers) if (l.onAir) n++;
  return n;
}

type Typing = (target: EventTarget | null) => boolean;

/** Elements that use Space / Enter themselves (and dialogs): Studio's TAKE key leaves them alone. */
const OWN_KEYS =
  "button,a[href],input,select,textarea,summary,[contenteditable],[role=button],[role=switch],[role=slider],[role=radio],[role=checkbox],[role=tab],[role^=menuitem],[role=option],[role=combobox],[role=listbox],[role=textbox],dialog,[role=dialog]";

export function keysTaken(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(OWN_KEYS) !== null;
}

export class ControlCtl {
  /** The saved Control layout (defaults until the project's ui-state loaded). */
  ui = $derived<ControlViewState>(uiState.value?.control ?? DEFAULT_CONTROL);

  /** The user's screens in the backend's order. */
  screens = $derived(live.list.filter((v) => isUserScreen(v.screen.id)));
  hidden = $derived(new Set(this.ui.hiddenScreenIds));
  visible = $derived(this.screens.filter((v) => !this.hidden.has(v.screen.id)));
  /** The saved selection when it is visible, else the first visible screen. */
  selectedId = $derived.by((): string | null => {
    const want = this.ui.selectedScreenId;
    if (want && this.visible.some((v) => v.screen.id === want)) return want;
    return this.visible[0]?.screen.id ?? null;
  });
  selected = $derived(this.selectedId ? (live.screens.get(this.selectedId) ?? null) : null);

  /** The selected screen's document (bindings, template ids); null until loaded. */
  doc = $state.raw<ScreenDoc | null>(null);
  /** Template id → summary (name, variant count). */
  templates = $state.raw<ReadonlyMap<string, TemplateSummary>>(new Map());
  sources = $state.raw<SourceSummary[]>([]);
  /** `sources` was loaded once (before that no source counts as missing). */
  sourcesLoaded = $state(false);
  /** The project's flows (cue buttons, the Automation strip). */
  readonly flows = new FlowsState();
  /** Opens the linear flow editor (lazy). */
  readonly flowEditor = new FlowEditorLauncher();
  /** Studio mode (Preview / Program). */
  readonly studio = new StudioCtl(() => this.ui.studio);

  /** Layer key → field → text typed but not yet confirmed (or still focused). */
  drafts = $state.raw<Readonly<Record<string, Readonly<Record<string, string>>>>>({});
  /** Expanded layer rows (layer keys). */
  expanded = $state.raw<ReadonlySet<string>>(new Set());
  /** The layer whose "Set hotkey" waits for a key. */
  capturing = $state<string | null>(null);

  /** Text sources the selected screen's layers are bound to (schedules have their recipe panel). */
  screenSources = $derived.by((): SourceSummary[] => {
    const id = this.selectedId;
    if (!id) return [];
    const bound = new Set<string>();
    if (this.doc?.id === id) {
      // A media layer has no bindings.
      for (const l of this.doc.layers as TemplateScreenLayer[]) for (const b of Object.values(l.bindings ?? {})) bound.add(b.sourceId);
    }
    return this.sources.filter((s) => s.kind !== "schedule" && (bound.has(s.id) || s.usedBy.includes(id)));
  });

  #focused: { key: string; field: string } | null = null;
  #docGen = 0;
  readonly #typing: Typing;
  readonly #edits = new LayerEdits(EDIT_DEBOUNCE_MS, (key, fields) => this.#sendFields(key, fields));

  constructor(typing: Typing = () => false) {
    this.#typing = typing;
  }

  // ---- Lifecycle ---------------------------------------------------------------------------------

  /** Mirrors, events and lists; returns the teardown (flushes pending edits). */
  connect(): () => void {
    const offLive = live.watch();
    const offServer = server.watch();
    const offFiles = on("project-files-changed", (c) => {
      if (c.change === "invalid") return;
      if (c.kind === "template") void this.loadTemplates();
      else if (c.kind === "source") void this.loadSources();
      else if (c.kind === "screen" && c.id === this.selectedId) void this.loadDoc(c.id);
    });
    const offSource = on("source-changed", (s) => this.putSource(s));
    const offFlows = this.flows.connect();
    void this.loadTemplates();
    void this.loadSources();
    return () => {
      offFlows();
      this.flowEditor.close();
      offSource();
      offFiles();
      this.capturing = null;
      void this.#edits.flushAll();
      offServer();
      offLive();
    };
  }

  async loadTemplates(): Promise<void> {
    try {
      const list = await listTemplates();
      this.templates = new Map(list.map((t) => [t.id, t]));
    } catch {
      // Names are cosmetic; rows show "Unknown template" until the next load.
    }
  }

  async loadSources(): Promise<void> {
    try {
      this.sources = await listSources();
      this.sourcesLoaded = true;
    } catch {
      // The strip stays as it was.
    }
  }

  /** Untracked: safe to call from an effect that should only rerun when `id` changes. */
  loadDoc(id: string | null): Promise<void> {
    return untrack(() => this.#loadDoc(id));
  }

  async #loadDoc(id: string | null): Promise<void> {
    const gen = ++this.#docGen;
    if (!id) {
      this.doc = null;
      return;
    }
    if (this.doc && this.doc.id !== id) this.doc = null;
    try {
      const state = await getScreen(id);
      if (gen === this.#docGen) this.doc = state.doc;
    } catch {
      // Removed meanwhile, or no backend: rows work without bindings and template names.
    }
  }

  // ---- Layout (uiState) --------------------------------------------------------------------------

  async #setControl(patch: Partial<ControlViewState>, what: string): Promise<void> {
    const v = uiState.value;
    if (!v) return;
    try {
      await uiState.update({ ...v, control: { ...v.control, ...patch } });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't ${what}: ${toAppError(e).message}` });
    }
  }

  select = (id: string): void => {
    if (id === this.ui.selectedScreenId) return;
    void this.#setControl({ selectedScreenId: id }, "select the screen");
  };

  setView = (view: ControlViewState["view"]): void => {
    if (view !== this.ui.view) void this.#setControl({ view }, "switch the view");
  };

  /** The right panel's width (clamped 280–640 server-side); resolves when saved or rolled back. */
  setPanelWidth = (panelWidth: number): Promise<void> =>
    panelWidth === this.ui.panelWidth ? Promise.resolve() : this.#setControl({ panelWidth }, "save the panel width");

  setColumns = (n: number): void => {
    const gridColumns = Math.min(GRID_COLUMNS.max, Math.max(GRID_COLUMNS.min, Math.round(n)));
    if (gridColumns !== this.ui.gridColumns) void this.#setControl({ gridColumns }, "change the columns");
  };

  setHidden = (id: string, hide: boolean): void => {
    const rest = this.ui.hiddenScreenIds.filter((x) => x !== id);
    void this.#setControl(
      { hiddenScreenIds: hide ? [...rest, id] : rest },
      hide ? "hide the screen" : "show the screen",
    );
  };

  showAll = (): void => {
    if (this.ui.hiddenScreenIds.length > 0) void this.#setControl({ hiddenScreenIds: [] }, "show the screens");
  };

  setLiveEdit = (liveEdit: ControlViewState["liveEdit"]): void => {
    if (liveEdit !== this.ui.liveEdit) void this.#setControl({ liveEdit }, "change live edit");
  };

  /** Automation off: no automatic trigger fires, auto-advance holds (the backend reacts on save). */
  setAutomationOff = (automationOff: boolean): void => {
    if (automationOff !== this.ui.automationOff)
      void this.#setControl({ automationOff }, automationOff ? "switch automation off" : "switch automation on");
  };

  // ---- On air ------------------------------------------------------------------------------------

  /** Optimistically replaces a layer; returns a rollback that only applies if nothing newer came. */
  #optimisticLayer(screenId: string, layer: LiveLayerView, patch: Partial<LiveLayerView>): () => void {
    const next = { ...layer, ...patch };
    live.putLayer(screenId, next);
    return () => {
      const now = live.screens.get(screenId)?.layers.find((l) => l.id === layer.id);
      if (now === next) live.putLayer(screenId, layer);
    };
  }

  #layer(screenId: string, layerId: string): LiveLayerView | null {
    return live.screens.get(screenId)?.layers.find((l) => l.id === layerId) ?? null;
  }

  /** Set by the media chunk (MediaTransport) once a media layer shows: the not-loaded question. */
  confirmMedia: ((layer: LiveLayerView) => Promise<boolean>) | null = null;

  take = async (screenId: string, layerId: string): Promise<void> => {
    const layer = this.#layer(screenId, layerId);
    if (!layer) return;
    // A layer whose media isn't loaded on every output asks once (the core never blocks a take;
    // without media to load the question answers yes at once).
    if (this.confirmMedia && !(await this.confirmMedia(layer))) return;
    const rollback = this.#optimisticLayer(screenId, layer, { onAir: true, pending: false });
    // TAKE plays what is typed right now.
    await this.#edits.flush(hotkeyKey(screenId, layerId));
    try {
      live.putLayer(screenId, await liveShow(screenId, layerId));
    } catch (e) {
      rollback();
      toasts.push({ kind: "error", message: `Couldn't take ${layer.name}: ${toAppError(e).message}` });
    }
  };

  clear = async (screenId: string, layerId: string): Promise<void> => {
    const layer = this.#layer(screenId, layerId);
    if (!layer) return;
    const rollback = this.#optimisticLayer(screenId, layer, { onAir: false, pending: false });
    try {
      live.putLayer(screenId, await liveHide(screenId, layerId));
    } catch (e) {
      rollback();
      toasts.push({ kind: "error", message: `Couldn't clear ${layer.name}: ${toAppError(e).message}` });
    }
  };

  /** Puts every given screen off air at once; returns a rollback for the untouched ones. */
  #optimisticOff(ids: readonly string[]): () => void {
    const before = new Map<string, LiveScreenView>();
    const after = new Map<string, LiveScreenView>();
    for (const id of ids) {
      const v = live.screens.get(id);
      if (!v || !v.layers.some((l) => l.onAir)) continue;
      const off = {
        ...v,
        layers: v.layers.map((l) => (l.onAir ? { ...l, onAir: false, pending: false } : l)),
      };
      before.set(id, v);
      after.set(id, off);
      live.put(off);
    }
    return () => {
      for (const [id, v] of before) if (live.screens.get(id) === after.get(id)) live.put(v);
    };
  }

  /** Clears one screen (animated out). */
  clearScreen = async (screenId: string): Promise<void> => {
    const name = live.screens.get(screenId)?.screen.name ?? "the screen";
    const rollback = this.#optimisticOff([screenId]);
    try {
      live.put(await liveClear(screenId));
    } catch (e) {
      rollback();
      toasts.push({ kind: "error", message: `Couldn't clear ${name}: ${toAppError(e).message}` });
    }
  };

  /** Clear every screen (animated out). */
  clearEvery = async (): Promise<void> => {
    const rollback = this.#optimisticOff([...live.screens.keys()]);
    try {
      for (const v of await liveClearAll()) live.put(v);
    } catch (e) {
      rollback();
      toasts.push({ kind: "error", message: `Couldn't clear the screens: ${toAppError(e).message}` });
    }
  };

  /** PANIC: everything off air at once, no out-animation. No confirm: it must be instant. */
  panic = async (): Promise<void> => {
    this.capturing = null;
    this.#optimisticOff([...live.screens.keys()]);
    try {
      for (const v of await livePanic()) live.put(v);
      toasts.push({ kind: "success", message: "PANIC: every screen is off air." });
    } catch (e) {
      // No rollback: what went off air stays off; reload the truth.
      void live.refresh();
      toasts.push({ kind: "error", message: `PANIC failed: ${toAppError(e).message}` });
    }
  };

  // ---- Fields ------------------------------------------------------------------------------------

  /** A layer's field values as shown: live values with local drafts on top. */
  values(screenId: string, layer: LiveLayerView): Readonly<Record<string, string>> {
    const d = this.drafts[hotkeyKey(screenId, layer.id)];
    return d ? { ...layer.fields, ...d } : layer.fields;
  }

  #setDraft(key: string, field: string, value: string | undefined) {
    const cur: Record<string, string> = { ...this.drafts[key] };
    if (value === undefined) delete cur[field];
    else cur[field] = value;
    const next = { ...this.drafts };
    if (Object.keys(cur).length === 0) delete next[key];
    else next[key] = cur;
    this.drafts = next;
  }

  #isFocused(key: string, field: string): boolean {
    return this.#focused?.key === key && this.#focused.field === field;
  }

  field = (screenId: string, layerId: string, field: string, value: string): void => {
    const key = hotkeyKey(screenId, layerId);
    this.#setDraft(key, field, value);
    this.#edits.push(key, field, value);
  };

  focus = (screenId: string, layerId: string, field: string): void => {
    this.#focused = { key: hotkeyKey(screenId, layerId), field };
  };

  blur = (screenId: string, layerId: string, field: string): void => {
    const key = hotkeyKey(screenId, layerId);
    if (this.#isFocused(key, field)) this.#focused = null;
    void this.#edits.flush(key).then(() => this.#settle(key, field));
  };

  /** Drops a draft once it is sent and no longer focused (the live value now carries it). */
  #settle(key: string, field: string) {
    if (this.#edits.pending(key) || this.#isFocused(key, field)) return;
    if (this.drafts[key]?.[field] !== undefined) this.#setDraft(key, field, undefined);
  }

  async #sendFields(key: string, fields: Record<string, string>): Promise<void> {
    const ids = splitHotkeyKey(key);
    if (!ids) return;
    try {
      live.putLayer(ids.screenId, await setLayerFields(ids.screenId, ids.layerId, fields));
    } catch (e) {
      // Show the saved value again, unless the user is still in the field.
      for (const f of Object.keys(fields)) if (!this.#isFocused(key, f)) this.#setDraft(key, f, undefined);
      toasts.push({ kind: "error", message: `Couldn't save the field: ${toAppError(e).message}` });
      return;
    }
    for (const f of Object.keys(fields)) this.#settle(key, f);
  }

  /** One timer state change (Start / Pause / Reset / Set); no debounce. */
  timer = async (screenId: string, layerId: string, key: string, action: TimerAction): Promise<void> => {
    try {
      live.putLayer(screenId, await setTimer(screenId, layerId, key, action));
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't change the timer: ${toAppError(e).message}` });
    }
  };

  toggleExpanded = (key: string): void => {
    const next = new Set(this.expanded);
    if (!next.delete(key)) next.add(key);
    this.expanded = next;
  };

  // ---- Sources -----------------------------------------------------------------------------------

  step = (sourceId: string, delta: number): Promise<void> => this.stepTo(sourceId, { to: "by", delta });

  /** Relative or absolute step (a slideshow's Next wraps with `{ to: "index", index: 0 }`). */
  stepTo = async (sourceId: string, step: SourceStep): Promise<void> => {
    try {
      this.putSource(await stepSource(sourceId, step));
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't step the source: ${toAppError(e).message}` });
    }
  };

  /** Replaces a summary (or adds a new source at the end); also the sources strip's API actions. */
  putSource = (s: SourceSummary): void => {
    const known = this.sources.some((x) => x.id === s.id);
    this.sources = known ? this.sources.map((x) => (x.id === s.id ? s : x)) : [...this.sources, s];
  };

  /** Settings remembered while auto-advance is off (so "Auto" turns back on with them). */
  #autoMemory = $state.raw<ReadonlyMap<string, AutoAdvance>>(new Map());

  #remember(sourceId: string, auto: AutoAdvance) {
    this.#autoMemory = new Map(this.#autoMemory).set(sourceId, auto);
  }

  /** The settings a source's auto-advance has, or would turn on with. */
  autoOf(s: SourceSummary): AutoAdvance {
    return s.auto ?? this.#autoMemory.get(s.id) ?? DEFAULT_AUTO;
  }

  /** Auto-advance on (with `autoOf`) or off. Optimistic; rolled back on failure. */
  setAutoOn = (sourceId: string, on: boolean): Promise<void> => {
    const s = this.sources.find((x) => x.id === sourceId);
    if (!s) return Promise.resolve();
    if (!on && s.auto) this.#remember(sourceId, s.auto);
    return this.#applyAuto(s, on ? this.autoOf(s) : null);
  };

  /** New settings: applied at once while on, remembered for the next "Auto" while off. */
  setAutoSettings = (sourceId: string, auto: AutoAdvance): Promise<void> => {
    const s = this.sources.find((x) => x.id === sourceId);
    if (!s) return Promise.resolve();
    this.#remember(sourceId, auto);
    return s.auto ? this.#applyAuto(s, auto) : Promise.resolve();
  };

  async #applyAuto(s: SourceSummary, auto: AutoAdvance | null): Promise<void> {
    const optimistic: SourceSummary = { ...s, auto, nextAt: auto ? s.nextAt : null };
    this.putSource(optimistic);
    try {
      this.putSource(await setAutoAdvance(s.id, auto));
    } catch (e) {
      if (this.sources.find((x) => x.id === s.id) === optimistic) this.putSource(s);
      toasts.push({ kind: "error", message: `Couldn't change auto-advance: ${toAppError(e).message}` });
    }
  }

  /** A layer of the selected screen's doc, read as a template layer (a media layer has no
   * bindings or template, so it reads as none). */
  #docLayer(layerId: string) {
    return (this.doc && this.doc.id === this.selectedId ? this.doc.layers.find((l) => l.id === layerId) : undefined) as
      | TemplateScreenLayer
      | undefined;
  }

  /** Field key → "Source · Column" (or the list binding's "all rows, sorted by …") for a layer of
   *  the selected screen (bound = read-only). */
  boundLabels(layerId: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const [key, b] of Object.entries(this.#docLayer(layerId)?.bindings ?? {})) {
      out[key] = bindingText(this.sources.find((x) => x.id === b.sourceId) ?? null, b);
    }
    return out;
  }

  /** The source a field of a layer of the selected screen is bound to. */
  boundSource(layerId: string, field: string): SourceSummary | null {
    const b = this.#docLayer(layerId)?.bindings?.[field];
    return b ? (this.sources.find((s) => s.id === b.sourceId) ?? null) : null;
  }

  /** The template of a layer of the selected screen (null until known). */
  templateOf(layerId: string): TemplateSummary | null {
    const layer = this.#docLayer(layerId);
    return layer ? (this.templates.get(layer.templateId) ?? null) : null;
  }

  // ---- Hotkeys -----------------------------------------------------------------------------------

  startCapture = (key: string): void => {
    this.capturing = this.capturing === key ? null : key;
  };

  /** Sets a layer's or cue's key through the binding table (the previous owner loses it). */
  setHotkey = (key: string, combo: string): void => {
    this.capturing = null;
    void this.#setKeys(key, [combo], "set the hotkey");
  };

  removeHotkey = (key: string): void => {
    const hotkeys = this.ui.hotkeys;
    const flow = key.startsWith("flow/") ? key.slice(5) : null;
    if (!(key in hotkeys) && !(flow !== null && `macro/${flow}` in hotkeys)) return;
    void this.#setKeys(key, null, "remove the hotkey");
  };

  async #setKeys(key: string, keys: string[] | null, what: string): Promise<void> {
    const id = commandOfProjectKey(key);
    if (!id) return;
    try {
      await setKeys(id, keys);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't ${what}: ${toAppError(e).message}` });
    }
  }

  /** A layer's Take / Clear toggle (its hotkey command): TAKE off air, CLEAR on air; in Studio
   * mode it cues the layer in or out of Preview instead. */
  toggle = async (screenId: string, layerId: string): Promise<void> => {
    const layer = this.#layer(screenId, layerId);
    if (!layer) return;
    if (this.studio.on) await this.studio.cue(screenId, layerId);
    else if (toggleDecision(layer) === "take") await this.take(screenId, layerId);
    else await this.clear(screenId, layerId);
  };

  /**
   * The page's window keydown (after the shell's, which runs the hotkey commands): "Set hotkey"
   * capture, and Studio mode's Space / Enter take. Cheap.
   */
  keydown = (e: KeyboardEvent): void => {
    if (this.capturing) {
      const key = this.capturing;
      const c = captureAction(e);
      if (c) {
        e.preventDefault();
        e.stopPropagation();
        if (c.kind === "cancel") this.capturing = null;
        else this.setHotkey(key, c.combo);
        return;
      }
    }
    const typing = this.#typing(e.target);
    if (this.studio.on && this.selectedId && isStudioTake(e, typing || keysTaken(e.target))) {
      e.preventDefault();
      void this.studio.take(this.selectedId);
    }
  };
}
