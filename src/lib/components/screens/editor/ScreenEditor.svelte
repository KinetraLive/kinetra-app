<!--
  ScreenEditor: the /screens/[id] page body, the screen layout editor. Full width:
    header     name (inline), mono resolution, background, mono screen URL + copy (LAN aware),
               save status, Duplicate, Delete
    left       LayersPanel (top = front; Play in/out, reorder, add from a template, delete)
    centre     Toolbar (lazy: snapping, rulers and guides, grid, safe areas, crop) over the Canvas
               (the only iframe: LivePreview of /screen/<id>, with draggable layer boxes, rulers,
               guides and overlays); the custom safe areas panel opens over it (lazy)
    right      Inspector for the selected layer (its own chunk, requested when this module loads,
               so the route's initial JS stays small); with several layers selected the
               MultiInspector (lazy: bounds, align, distribute)

  Props:
    id: string   the screen id (remount the component to switch screens)

  Saving: every edit replaces the working copy (`doc`) and touches the Autosave (500 ms after the
  last edit, one save in flight, the latest doc queued; Ctrl/Cmd+S saves at once; unmount flushes).
  `saveScreen` returns the live state (on air, pending) and warnings, which replace ours.

  Dragging: the canvas reports the dragged rects per animation frame; `setLayerRect` (memory +
  broadcast, no disk) is sent once per dragged layer per update, throttled so all of them together
  stay <= 20/s, with a guaranteed trailing update; the drop commits the rects to the doc, so it is
  saved once 500 ms later. Align / distribute and nudges commit the same way.

  Selection: `selection` (ids); Shift/Ctrl+click on the canvas or in the layers panel adds or
  removes, a marquee on empty canvas selects, Ctrl+A selects all. One layer: the Inspector; more:
  the MultiInspector. A layer's aspect lock lives for the editor session.

  Layout tools (editor-only, never sent to the renderer): view options (snapping, guides, grid +
  size, safe overlays, crop) are in the project's ui-state (`screenEditor`, optimistic through
  the uiState store); ruler guides and custom safe areas are part of the screen file (`guides`,
  `safeAreas`) and autosave like any edit. The editor's commands (toggles, align, distribute,
  select all, clear guides) are registered with the command palette while it is mounted.

  Changed on disk (`project-files-changed`): this screen without unsaved edits reloads silently
  (toast); with unsaved edits a DiskBanner offers Reload / Keep mine (autosave waits). Deleted on
  disk: a banner whose Close goes back to /screens. Invalid: a toast; the working copy stays. A
  template used here that changed on disk is refetched silently.

  Source bindings: the source list (listSources) loads lazily: at load when the screen has
  bindings, else when a Bind menu first opens; it is refreshed when a source changes on disk.
  Bind / Unbind edit `layer.bindings` and autosave like any edit; switching a layer's template
  drops bindings for fields the new template doesn't have. A list field binds a whole source with
  sort / limit.

  Variants: the Inspector's Variant select calls setLayerVariant (crossfade; on air it follows Live
  edit) after flushing the autosave; the result replaces the live state and warnings, and the
  working copy takes the variant without another save. While mounted, the palette lists "Switch
  <layer> to <variant>" for every layer whose template has several (a lazy chunk).

  Timers: the Inspector's timer controls call setTimer (the server applies the action, saves the
  screen and puts it on air at once). The returned value replaces the field in the working copy
  without an autosave, so a later save carries it instead of the old state; live-changed does the
  same for timers changed elsewhere (Control, countdown end).
-->
<script module lang="ts">
  // The Inspector (fields, variables, bindings, list and timer controls) is a chunk of its own,
  // requested at once: it is needed only after a layer is selected.
  const inspectorChunk = import("./Inspector.svelte");
  // The toolbar over the canvas: requested at once too (it fills a reserved row).
  const toolbarChunk = import("./Toolbar.svelte");
  // The layers panel likewise (it fills the left column; requested with this module, so it shows
  // with the canvas).
  const layersChunk = import("./LayersPanel.svelte");
  // Only with several layers selected / once the safe areas panel opens.
  let multiChunk: Promise<typeof import("./MultiInspector.svelte")> | null = null;
  const loadMulti = () => (multiChunk ??= import("./MultiInspector.svelte"));
  let safePanelChunk: Promise<typeof import("./SafeAreasPanel.svelte")> | null = null;
  const loadSafePanel = () => (safePanelChunk ??= import("./SafeAreasPanel.svelte"));

  const NO_GUIDES = { x: [], y: [] };
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import ArrowLeft from "lucide-svelte/icons/arrow-left";
  import Copy from "lucide-svelte/icons/copy";
  import Link from "lucide-svelte/icons/link";
  import RotateCw from "lucide-svelte/icons/rotate-cw";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import { goto } from "$app/navigation";
  import { on, toAppError } from "$lib/api";
  import { provideContext, registerCommands, registerProvider } from "$lib/commands/registry.svelte";
  import * as live from "$lib/api/live";
  import * as screensApi from "$lib/api/screens";
  import * as templatesApi from "$lib/api/templates";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import Select from "$lib/components/Select.svelte";

  import { GalleryLauncher, type GalleryLayer } from "$lib/components/templates/gallery/launcher.svelte";
  import type { UnlinkCopy } from "$lib/components/sources/link";
  import { pickerHex } from "$lib/components/templates/editor/model";
  import { Autosave, diskReaction, type SaveState } from "$lib/components/templates/editor/sync";
  import { copyUrl, lanScreenUrl, screenUrl, server, statusText } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  // Imported together with the project store so both stay in their one shared chunk.
  import { project } from "$lib/stores/project.svelte";
  import { uiState } from "$lib/stores/uiState.svelte";
  import {
    SCREEN_NAME_MAX,
    type ContentSizing,
    type FilesChanged,
    type LayerAnchor,
    type LiveLayerView,
    type Rect,
    type SafeArea,
    type ScreenDoc,
    type ScreenEditorView,
    type ScreenGuides,
    type ScreenLayer,
    type ScreenState,
    type ScreenWarning,
    type SourceBinding,
    type TemplateScreenLayer,
    type SourceSummary,
    type TemplateDoc,
    type TemplateSummary,
    type TimerAction,
  } from "$lib/types";
  import type { AlignKind } from "./align";
  import Canvas, { type LayerRect } from "./Canvas.svelte";
  import { Throttle, aspectLock, clampRect, moveGroup, sameRect } from "./geometry";
  import { DEFAULT_SCREEN_EDITOR } from "./limits";
  import { setAnchors } from "./anchor";
  import type { LayerRow } from "./LayersPanel.svelte";
  import {
    addLayer,
    bindField,
    blockedReason,
    canAddLayer,
    hasBindings,
    isBackground,
    moveLayer,
    newLayer,
    newLayerId,
    panelToDoc,
    patchLayer,
    removeLayer,
    resizedToFit,
    screenNameError,
    setGuides,
    setLayerSizing,
    setRects,
    setSafeAreas,
    shiftLayer,
    switchTemplate,
    unbindField,
  } from "./model";

  let { id }: { id: string } = $props();

  /** Autosave delay; also "save 500 ms after the drop". */
  const SAVE_MS = 500;
  /** setLayerRect at most every 50 ms (<= 20/s) while dragging. */
  const RECT_INTERVAL_MS = 50;

  const uid = $props.id();

  // ---- State --------------------------------------------------------------------------------------

  let loaded = $state(false);
  let loadError = $state<string | null>(null);

  /** The working copy; replaced wholesale on every edit. */
  let doc = $state.raw<ScreenDoc | null>(null);
  let updatedAt = $state<string | null>(null);
  let liveLayers = $state.raw<Record<string, LiveLayerView>>({});
  let warnings = $state.raw<ScreenWarning[]>([]);
  let summaries = $state.raw<TemplateSummary[]>([]);
  let templates = $state.raw<Map<string, TemplateDoc>>(new Map());
  /** Selected layer ids, in the order they were selected. */
  let selection = $state.raw<string[]>([]);
  const selectedId = $derived(selection.length === 1 ? selection[0] : null);
  // The command context: this screen and its selected layers.
  $effect(() => provideContext({ screenId: id, layerIds: selection }));
  let playBusy = $state.raw<string[]>([]);

  const summaryById = $derived(new Map(summaries.map((t) => [t.id, t])));
  const missingIds = $derived(
    new Set(warnings.filter((w) => w.code === "templateMissing" || w.code === "assetMissing").map((w) => w.layerId)),
  );
  const isMissing = (l: ScreenLayer) =>
    missingIds.has(l.id) || (l.kind !== "media" && summaries.length > 0 && !summaryById.has(l.templateId));
  /** Template ids the layers use. */
  const tids = (ls: readonly ScreenLayer[]) => ls.flatMap((l) => (l.kind !== "media" ? [l.templateId] : []));
  /** Layer id → its sourceMissing messages. */
  const sourceWarnings = $derived.by(() => {
    const m = new Map<string, string[]>();
    for (const w of warnings) if (w.code === "sourceMissing") m.set(w.layerId, [...(m.get(w.layerId) ?? []), w.message]);
    return m;
  });

  const rows = $derived<LayerRow[]>(
    (doc?.layers ?? [])
      .map((l) => ({
        id: l.id,
        name: l.name,
        templateName:
          l.kind === "media"
            ? l.media.asset
            : (summaryById.get(l.templateId)?.name ?? templates.get(l.templateId)?.name ?? null),
        media: l.kind === "media",
        onAir: liveLayers[l.id]?.onAir ?? false,
        pending: liveLayers[l.id]?.pending ?? false,
        missing: isMissing(l),
        sourceMissing: sourceWarnings.has(l.id),
      }))
      .reverse(),
  );
  const selected = $derived(doc?.layers.find((l) => l.id === selectedId) ?? null);
  const selectedLayers = $derived(doc ? doc.layers.filter((l) => selection.includes(l.id)) : []);

  function applyState(st: ScreenState) {
    liveLayers = Object.fromEntries(st.live.layers.map((l) => [l.id, l]));
    warnings = st.warnings;
  }

  function setLive(v: LiveLayerView) {
    liveLayers = { ...liveLayers, [v.id]: v };
  }

  function fail(e: unknown) {
    toasts.push({ kind: "error", message: toAppError(e).message });
  }

  // ---- Templates ----------------------------------------------------------------------------------

  /** Loads template docs not cached yet (missing ones are skipped; the warnings say so). */
  async function ensureTemplates(ids: Iterable<string>, refresh = false): Promise<void> {
    const want = [...new Set(ids)].filter((t) => refresh || !templates.has(t));
    if (want.length === 0) return;
    const got = await Promise.all(want.map((t) => templatesApi.getTemplate(t).catch(() => null)));
    const next = new Map(templates);
    want.forEach((t, i) => {
      const d = got[i];
      if (d) next.set(t, d);
      else next.delete(t);
    });
    templates = next;
  }

  async function refreshSummaries() {
    try {
      summaries = await templatesApi.listTemplates();
    } catch {
      // Keep the last list; names fall back to the loaded docs.
    }
  }

  // ---- Sources (for bindings; loaded lazily) --------------------------------------------------------

  /** Null until first needed. */
  let sources = $state.raw<SourceSummary[] | null>(null);
  let sourcesLoading: Promise<void> | null = null;
  /** The sources commands: a small chunk, loaded with the first need (keeps the route small). */
  const sourcesApi = () => import("$lib/api/sources");

  /** Loads the source list once (or again with `refresh`); failures keep what we have. */
  function loadSources(refresh = false): Promise<void> {
    if (sourcesLoading) return sourcesLoading;
    if (sources !== null && !refresh) return Promise.resolve();
    sourcesLoading = sourcesApi().then((api) => api.listSources()).then(
      (list) => {
        sources = list;
        sourcesLoading = null;
      },
      (e: unknown) => {
        sourcesLoading = null;
        if (sources === null) sources = [];
        fail(e);
      },
    );
    return sourcesLoading;
  }

  // ---- Save ---------------------------------------------------------------------------------------

  let saveState = $state<SaveState>({ kind: "saved", at: null });
  let disk = $state<"conflict" | "removed" | null>(null);
  let diskBusy = $state(false);
  let deleted = false;

  const docBlocked = $derived(doc ? blockedReason(doc, templates) : null);
  const saveBlocked = $derived(
    disk === "conflict" ? "Changed on disk: choose Reload or Keep mine." : docBlocked,
  );

  const autosave = new Autosave<ScreenDoc>({
    delayMs: SAVE_MS,
    snapshot: () => ({ ...(doc as ScreenDoc), updatedAt: updatedAt ?? "" }),
    blocked: () => saveBlocked,
    save: async (d) => {
      const st = await screensApi.saveScreen(d);
      applyState(st);
      return { updatedAt: st.doc.updatedAt };
    },
    onState: (s) => {
      if (s.kind === "saved" && s.at) updatedAt = s.at;
      if (s.kind !== saveState.kind || s.kind === "saved" || s.kind === "failed") saveState = s;
    },
    describe: (e) => toAppError(e).message,
  });

  function edit(next: ScreenDoc) {
    doc = next;
    autosave.touch();
  }

  function saveNow() {
    if (loaded && !deleted) void autosave.saveNow();
  }

  function onkeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "s") {
      e.preventDefault();
      saveNow();
    }
  }

  // ---- Drag ---------------------------------------------------------------------------------------

  let dragging = false;
  /** One setLayerRect per layer per update; the interval grows with the count (<= 20/s in all). */
  const rectSender = new Throttle<LayerRect[]>(RECT_INTERVAL_MS, (changes) => {
    for (const c of changes) {
      screensApi.setLayerRect(id, c.id, c.rect).then(
        (v) => setLive(v),
        () => {
          // The drop saves the final rect; a failed live update is not worth a toast per frame.
        },
      );
    }
  });

  /** A resize's switch to Scale to fit being saved: its live rects wait for it (latest wins). */
  let fitSave: Promise<void> | null = null;
  let heldRects: LayerRect[] | null = null;

  function sendRects(changes: LayerRect[]) {
    if (fitSave) {
      heldRects = changes;
      return;
    }
    rectSender.intervalMs = RECT_INTERVAL_MS * Math.max(1, changes.length);
    rectSender.call(changes);
  }

  function onDrag(changes: LayerRect[]) {
    dragging = true;
    sendRects(changes);
  }

  /**
   * End of a drag, a nudge, an align or a typed rect: live now (a drag only flushes its trailing
   * update, which already carries these rects; anything else goes through the same throttle, so a
   * held arrow key's repeats stay <= 20/s), saved once 500 ms later.
   */
  function commitRects(changes: LayerRect[], resized = false) {
    if (dragging && !fitSave) rectSender.flush();
    else sendRects(changes);
    dragging = false;
    if (!doc) return;
    const next = setRects(doc, changes);
    edit(resized ? resizedToFit(next, changes.map((c) => c.id)) : next);
  }

  /**
   * A resize drag began: a layer that isn't Scale to fit or explicitly Reflow becomes Scale to fit
   * now (saved at once, before the drag's live rects, so the output scales while dragging; the
   * drop saves the box).
   */
  function onResizeStart(layerId: string) {
    if (!doc) return;
    const next = resizedToFit(doc, [layerId]);
    if (next === doc) return;
    edit(next);
    const p = autosave.saveNow().finally(() => {
      if (fitSave !== p) return;
      fitSave = null;
      const held = heldRects;
      heldRects = null;
      if (held) sendRects(held);
    });
    fitSave = p;
  }

  /** Commits `rects` (same order as `layers`) for the layers whose box changed. */
  function applyRects(layers: readonly ScreenLayer[], rects: readonly Rect[]) {
    if (!doc) return;
    const size = { w: doc.width, h: doc.height };
    const changes = layers
      .map((l, i) => ({ id: l.id, rect: clampRect(rects[i], size) }))
      .filter((c, i) => !sameRect(c.rect, layers[i].rect));
    if (changes.length > 0) commitRects(changes);
  }

  // ---- Selection, align, distribute -------------------------------------------------------------

  function selectRow(layerId: string, additive: boolean) {
    if (!additive) selection = [layerId];
    else selection = selection.includes(layerId) ? selection.filter((x) => x !== layerId) : [...selection, layerId];
  }

  async function alignSelection(kind: AlignKind) {
    const { align } = await import("./align");
    if (!doc || selectedLayers.length === 0) return;
    const sel = selectedLayers;
    applyRects(sel, align(sel.map((l) => l.rect), kind, { w: doc.width, h: doc.height }));
  }

  async function distributeSelection(axis: "x" | "y") {
    const { distribute } = await import("./align");
    if (!doc || selectedLayers.length < 3) return;
    const sel = selectedLayers;
    applyRects(sel, distribute(sel.map((l) => l.rect), axis));
  }

  function moveSelection(dx: number, dy: number) {
    if (!doc) return;
    const sel = selectedLayers;
    applyRects(sel, moveGroup(sel.map((l) => l.rect), dx, dy, { w: doc.width, h: doc.height }));
  }

  /**
   * Anchors the given layers (`patch` on top of each one's anchor; each placed on its own by the
   * TS mirror of the core's math, so the canvas moves at once) and saves now so the output
   * follows. The core places them again on save (the authority).
   */
  function anchorLayers(ids: readonly string[], patch: Partial<LayerAnchor>) {
    if (!doc) return;
    edit(setAnchors(doc, ids, patch));
    saveNow();
  }

  /** Content sizing of the given layers (one edit, one save; the core applies the live rules). */
  function setSizing(ids: readonly string[], sizing: ContentSizing) {
    if (doc) edit(setLayerSizing(doc, ids, sizing));
  }

  /** "Fit box to graphic": each layer's design box with Scale to fit (one save). */
  function fitBoxes(changes: LayerRect[]) {
    commitRects(changes);
    setSizing(changes.map((c) => c.id), "fit");
  }

  /** Aspect locks the user toggled, per layer id, for this editor session (else the default). */
  let lockToggles = $state.raw<ReadonlyMap<string, boolean>>(new Map());
  /** Layers whose aspect is locked: toggled, else on for layers that scale (`aspectLock`). */
  const locked = $derived<ReadonlySet<string>>(
    new Set(
      (doc?.layers ?? [])
        // Media layers have no sizing: their aspect is locked by default too.
        .filter((l) => aspectLock((l as TemplateScreenLayer).sizing, lockToggles.get(l.id)))
        .map((l) => l.id),
    ),
  );

  function toggleLock(layerId: string | null = selectedId) {
    if (!layerId) return;
    lockToggles = new Map(lockToggles).set(layerId, !locked.has(layerId));
  }

  // ---- View options, guides, safe areas -----------------------------------------------------------

  /** Used only while no project (and so no ui-state) is loaded. */
  let localView = $state.raw<ScreenEditorView>(DEFAULT_SCREEN_EDITOR);
  const projectView = $derived(project.current ? uiState.value : null);
  const view = $derived<ScreenEditorView>(
    projectView ? { ...DEFAULT_SCREEN_EDITOR, ...projectView.screenEditor } : localView,
  );

  /** Optimistic; rolled back (with a toast) if the backend refuses. */
  function setView(patch: Partial<ScreenEditorView>) {
    const next = { ...view, ...patch };
    const v = projectView;
    if (!v) {
      localView = next;
      return;
    }
    uiState.update({ ...v, screenEditor: next }).catch(fail);
  }

  const guides = $derived<ScreenGuides>(doc?.guides ?? NO_GUIDES);

  function editGuides(next: ScreenGuides) {
    if (doc) edit(setGuides(doc, next));
  }

  function editSafeAreas(next: SafeArea[]) {
    if (doc) edit(setSafeAreas(doc, next));
  }

  let safePanel = $state(false);

  // ---- Layers -------------------------------------------------------------------------------------

  async function addFromTemplate(templateId: string) {
    if (!doc || !canAddLayer(doc)) return;
    try {
      await ensureTemplates([templateId]);
    } catch (e) {
      fail(e);
      return;
    }
    const t = templates.get(templateId);
    if (!t || !doc) {
      toasts.push({ kind: "error", message: "That template couldn't be loaded." });
      return;
    }
    const layer = newLayer(t, doc);
    edit(addLayer(doc, layer));
    selection = [layer.id];
  }

  /** "Media…": a media layer showing the chosen file (its own size centered, or the screen). */
  async function addMedia() {
    if (!doc || !canAddLayer(doc)) return;
    try {
      const layer = await (await import("$lib/components/media/pick")).pickMediaLayer(doc, newLayerId);
      if (!layer || !doc || !canAddLayer(doc)) return;
      edit(addLayer(doc, layer));
      selection = [layer.id];
    } catch (e) {
      fail(e);
    }
  }

  function deleteLayers(ids: readonly string[]) {
    if (!doc) return;
    const i = doc.layers.findIndex((l) => ids.includes(l.id));
    if (i < 0) return;
    let next = doc;
    for (const layerId of ids) next = removeLayer(next, layerId);
    const kept = selection.filter((x) => !ids.includes(x));
    if (kept.length !== selection.length) {
      // Select the neighbour below in the panel (behind), else the one above.
      const neighbour = next.layers[Math.max(0, i - 1)]?.id;
      selection = kept.length > 0 ? kept : neighbour ? [neighbour] : [];
    }
    edit(next);
  }

  function shift(layerId: string, dir: 1 | -1) {
    if (!doc) return;
    const next = shiftLayer(doc, layerId, dir);
    if (next !== doc) edit(next);
  }

  function reorder(layerId: string, toRow: number) {
    if (!doc) return;
    const n = doc.layers.length;
    const from = doc.layers.findIndex((l) => l.id === layerId);
    const next = moveLayer(doc, from, panelToDoc(n, toRow));
    if (next !== doc) edit(next);
  }

  async function play(layerId: string, show: boolean) {
    if (playBusy.includes(layerId)) return;
    playBusy = [...playBusy, layerId];
    const prev = liveLayers[layerId];
    if (prev) setLive({ ...prev, onAir: show }); // optimistic
    try {
      await autosave.flush(); // play what the editor shows (and save a new layer first)
      const v = show ? await live.liveShow(id, layerId) : await live.liveHide(id, layerId);
      setLive(v);
    } catch (e) {
      if (prev) setLive(prev);
      fail(e);
    } finally {
      playBusy = playBusy.filter((x) => x !== layerId);
    }
  }

  // ---- Inspector ----------------------------------------------------------------------------------

  function patchSelected(p: Partial<ScreenLayer>) {
    if (doc && selectedId) edit(patchLayer(doc, selectedId, p));
  }

  async function changeTemplate(templateId: string) {
    const layerId = selectedId;
    if (!doc || !layerId) return;
    await ensureTemplates([templateId]);
    const t = templates.get(templateId);
    const l = doc?.layers.find((x) => x.id === layerId);
    if (!t || (!l || l.kind === "media") || !doc) {
      toasts.push({ kind: "error", message: "That template couldn't be loaded." });
      return;
    }
    const prev = templates.get(l.templateId) ?? summaryById.get(l.templateId) ?? null;
    edit(patchLayer(doc, layerId, switchTemplate(l, prev, t)));
  }

  function setField(key: string, value: string) {
    if ((!!selected && selected.kind !== "media")) patchSelected({ fields: { ...selected.fields, [key]: value } });
  }

  function setVar(key: string, value: string) {
    if ((!!selected && selected.kind !== "media")) patchSelected({ vars: { ...selected.vars, [key]: value } });
  }

  function bind(key: string, binding: SourceBinding) {
    if (doc && selectedId) edit(bindField(doc, selectedId, key, binding));
  }

  /**
   * Unbinds field `key`. Unlinking a list from a data table keeps a copy of the table's rows (as
   * the list showed them) as the layer's own value.
   */
  async function unbind(key: string) {
    if (!doc || !selectedId) return;
    const layerId = selectedId;
    const layer = doc.layers.find((l) => l.id === layerId);
    if ((!layer || layer.kind === "media")) return;
    const field = templates.get(layer.templateId)?.fields.find((f) => f.key === key);
    let copy: UnlinkCopy | null = null;
    if (field?.type === "list") {
      try {
        const { unlinkedValue } = await import("$lib/components/sources/link");
        copy = await unlinkedValue(layer.bindings?.[key], field.columns ?? [], sources, async (id) => (await sourcesApi()).getSource(id));
      } catch (e) {
        toasts.push({ kind: "error", message: `Couldn't copy the table's rows: ${toAppError(e).message}` });
        return;
      }
    }
    if (!doc) return;
    let next = unbindField(doc, layerId, key);
    const l = next.layers.find((x) => x.id === layerId);
    if (copy && (!!l && l.kind !== "media")) {
      next = patchLayer(next, layerId, { fields: { ...l.fields, [key]: copy.value } });
      if (copy.note) toasts.push({ kind: "info", message: copy.note });
    }
    if (next !== doc) edit(next);
  }

  /** Timer field keys per template id (for syncing server-side timer changes). */
  function timerKeys(templateId: string): string[] {
    return (templates.get(templateId)?.fields ?? []).filter((f) => f.type === "timer").map((f) => f.key);
  }

  /**
   * Takes the server's timer values into the working copy (no autosave: the server saved them).
   * Only fields that differ are replaced.
   */
  function syncTimers(layerId: string, fields: Readonly<Record<string, string>>) {
    const l = doc?.layers.find((x) => x.id === layerId);
    if (!doc || (!l || l.kind === "media")) return;
    let next: Record<string, string> | null = null;
    for (const key of timerKeys(l.templateId)) {
      const v = fields[key];
      if (v !== undefined && v !== l.fields[key]) (next ??= { ...l.fields })[key] = v;
    }
    if (next) doc = patchLayer(doc, layerId, { fields: next });
  }

  async function switchVariant(layerId: string, variant: string) {
    if (!doc) return;
    try {
      await autosave.flush(); // a new layer must exist on the server first
      const prev = (doc.layers.find((l) => l.id === layerId) as TemplateScreenLayer | undefined)?.variant;
      doc = patchLayer(doc, layerId, { variant }); // the working copy carries it; no save needed
      try {
        const st = await screensApi.setLayerVariant(id, layerId, variant, "crossfade");
        applyState(st);
        updatedAt = st.doc.updatedAt;
      } catch (e) {
        if ((doc?.layers.find((l) => l.id === layerId) as TemplateScreenLayer | undefined)?.variant === variant) doc = patchLayer(doc, layerId, { variant: prev });
        throw e;
      }
    } catch (e) {
      fail(e);
    }
  }

  /** The commands chunk, once loaded (see onMount): it keeps "Switch <layer> to <variant>" current. */
  let commandsMod = $state.raw<typeof import("./commands") | null>(null);
  $effect(() => {
    const m = commandsMod;
    if (m) return m.watchVariantCommands(() => m.docVariantLayers(id, doc, templates), { section: "edit", keyScope: "screenEditor" }, (l, v) => switchVariant(l.layerId, v));
  });

  async function timer(key: string, action: TimerAction) {
    const layerId = selectedId;
    if (!layerId) return;
    try {
      await autosave.flush(); // a new layer must exist on the server first
      const { setTimer } = await import("$lib/api/automation");
      const v = await setTimer(id, layerId, key, action);
      setLive(v);
      syncTimers(layerId, v.fields);
    } catch (e) {
      fail(e);
    }
  }

  function resetVar(key: string) {
    if ((!selected || selected.kind === "media")) return;
    const vars = { ...selected.vars };
    delete vars[key];
    patchSelected({ vars });
  }

  // ---- Header -------------------------------------------------------------------------------------

  $effect(() => server.watch());
  const status = $derived(server.status);
  const localUrl = $derived(screenUrl(status, id));
  const shownUrl = $derived(lanScreenUrl(status, id) ?? localUrl);

  let nameDraft = $state("");
  let nameError = $state<string | null>(null);

  async function commitName() {
    if (!doc) return;
    const next = nameDraft.trim();
    if (next === doc.name) {
      nameDraft = next;
      nameError = null;
      return;
    }
    nameError = screenNameError(next);
    if (nameError) return;
    const prev = doc.name;
    doc = { ...doc, name: next }; // optimistic; later saves carry it
    try {
      const summary = await screensApi.renameScreen(id, next);
      if (doc) doc = { ...doc, name: summary.name };
      nameDraft = summary.name;
      updatedAt = summary.updatedAt;
      // A save that started before the rename carries the old name: save once more.
      if (autosave.saving) autosave.touch();
    } catch (e) {
      if (doc) doc = { ...doc, name: prev };
      nameDraft = prev;
      fail(e);
    }
  }

  function onNameKey(e: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (e.key === "Enter") {
      e.preventDefault();
      void commitName();
    } else if (e.key === "Escape") {
      nameDraft = doc?.name ?? "";
      nameError = null;
      e.currentTarget.blur();
    }
  }

  // Background: "transparent" or a hex colour. The last colour is remembered while transparent.
  let lastColour = $state("#000000");
  let bgDraft = $state("");
  const bgMode = $derived(doc?.background === "transparent" ? "transparent" : "colour");
  const BG_OPTIONS = [
    { value: "transparent" as const, label: "Transparent" },
    { value: "colour" as const, label: "Color" },
  ];

  function setBackground(v: string) {
    if (!doc) return;
    bgDraft = v;
    if (v !== "transparent" && isBackground(v)) lastColour = v;
    edit({ ...doc, background: v });
  }

  function setBgMode(mode: "transparent" | "colour") {
    setBackground(mode === "transparent" ? "transparent" : lastColour);
  }

  // ---- Duplicate / delete -------------------------------------------------------------------------

  let dupBusy = $state(false);

  async function duplicate() {
    if (dupBusy || !doc) return;
    dupBusy = true;
    try {
      await autosave.flush(); // the copy is made from what is saved
      const copy = await screensApi.duplicateScreen(id, `${doc.name} copy`.slice(0, SCREEN_NAME_MAX));
      toasts.push({ kind: "success", message: `Duplicated as “${copy.doc.name}”` });
      await goto(`/screens/${copy.doc.id}`);
    } catch (e) {
      fail(e);
    } finally {
      dupBusy = false;
    }
  }

  type ConfirmDialogT = typeof import("$lib/components/ConfirmDialog.svelte").default;
  let Confirm = $state<ConfirmDialogT | null>(null);
  let confirmOpen = $state(false);
  let deleteBusy = $state(false);

  async function askDelete() {
    if (!Confirm) {
      try {
        Confirm = (await import("$lib/components/ConfirmDialog.svelte")).default;
      } catch (e) {
        fail(e);
        return;
      }
    }
    confirmOpen = true;
  }

  async function confirmDelete() {
    deleteBusy = true;
    try {
      autosave.cancel();
      deleted = true;
      await screensApi.deleteScreen(id);
      confirmOpen = false;
      toasts.push({ kind: "success", message: `Deleted “${doc?.name ?? ""}”` });
      await goto("/screens");
    } catch (e) {
      deleted = false;
      fail(e);
    } finally {
      deleteBusy = false;
    }
  }

  // ---- Gallery ("New template…") -----------------------------------------------------------------

  const gallery = new GalleryLauncher();
  /** "Browse styles…" opens it on a layer; null for "New template…". */
  let styleLayer = $state.raw<GalleryLayer | null>(null);
  const showGallery = (layer: GalleryLayer | null) => {
    styleLayer = layer;
    void gallery.show();
  };

  // ---- Load, external changes, teardown -------------------------------------------------------------

  function applyDoc(st: ScreenState) {
    doc = st.doc;
    updatedAt = st.doc.updatedAt;
    nameDraft = st.doc.name;
    nameError = null;
    bgDraft = st.doc.background;
    if (st.doc.background !== "transparent" && isBackground(st.doc.background)) lastColour = st.doc.background;
    applyState(st);
    saveState = { kind: "saved", at: st.doc.updatedAt };
    if (selection.some((x) => !st.doc.layers.some((l) => l.id === x))) {
      selection = selection.filter((x) => st.doc.layers.some((l) => l.id === x));
    }
  }

  async function reloadFromDisk(): Promise<boolean> {
    let st: ScreenState;
    try {
      st = await screensApi.getScreen(id);
    } catch (e) {
      fail(e);
      return false;
    }
    autosave.discard();
    rectSender.cancel();
    applyDoc(st);
    disk = null;
    await ensureTemplates(tids(st.doc.layers), true).catch(() => {});
    return true;
  }

  function onFilesChanged(e: FilesChanged) {
    if (!loaded || deleted || !doc) return;
    if (e.kind === "template") {
      if (tids(doc.layers).includes(e.id) && e.change !== "invalid") {
        void ensureTemplates([e.id], true).catch(() => {});
      }
      if (e.change !== "invalid") void refreshSummaries();
      return;
    }
    if (e.kind === "source") {
      if (sources !== null && e.change !== "invalid") void loadSources(true);
      return;
    }
    const reaction = diskReaction(
      e,
      "screen",
      id,
      autosave.dirty || autosave.saving || dragging || disk === "conflict",
    );
    switch (reaction) {
      case "reload":
        void reloadFromDisk().then((ok) => {
          if (ok) toasts.push({ kind: "info", message: `“${doc?.name ?? ""}” changed on disk and was reloaded.` });
        });
        break;
      case "conflict":
        disk = "conflict";
        break;
      case "removed":
        deleted = true;
        autosave.cancel();
        rectSender.cancel();
        disk = "removed";
        break;
      case "invalid":
        toasts.push({
          kind: "error",
          message: `“${doc.name}” changed on disk but couldn't be loaded: ${e.message ?? "invalid file"}. Your copy here is kept.`,
        });
        break;
    }
  }

  async function diskReload() {
    diskBusy = true;
    try {
      await reloadFromDisk();
    } finally {
      diskBusy = false;
    }
  }

  async function diskKeep() {
    disk = null;
    diskBusy = true;
    try {
      await autosave.saveNow();
    } finally {
      diskBusy = false;
    }
  }

  onMount(() => {
    let alive = true;
    const offFiles = on("project-files-changed", onFilesChanged);
    // On air, staged and bound values change from anywhere (Control, auto-out, source steps).
    const offLive = on("live-changed", (view) => {
      if (!alive || view.screen.id !== id) return;
      liveLayers = Object.fromEntries(view.layers.map((l) => [l.id, l]));
      for (const l of view.layers) syncTimers(l.id, l.fields);
      if (sources !== null && doc && hasBindings(doc)) void loadSources(true);
    });
    // The editor's palette commands (and their shortcuts) exist only while it is mounted; their
    // module is a lazy chunk shared with the toolbar.
    let offCommands = () => {};
    let offStyles = () => {};
    void import("./commands").then((mod) => {
      if (!alive) return;
      commandsMod = mod;
      // Any style change (palette, gallery) or update to a template's latest look saves this
      // editor first and reloads it (and the template list: variants, newer looks) after.
      offStyles = mod.styleHook(() => autosave.flush(), () => void reloadFromDisk().then(refreshSummaries));
      const { editorCommands, editorProvider, renameCommand } = mod;
      const items = {
        layers: () => doc?.layers ?? [],
        select: (layerId: string) => (selection = [layerId]),
        name: () => doc?.name ?? "",
        rename: (name: string) => {
          nameDraft = name;
          return commitName();
        },
      };
      const offProvider = registerProvider(editorProvider(items));
      const offRename = registerCommands([renameCommand(items)]);
      const offEditor = registerCommands(
      editorCommands({
        view: () => view,
        setView,
        selectionSize: () => selectedLayers.length,
        layerCount: () => doc?.layers.length ?? 0,
        hasGuides: () => guides.x.length + guides.y.length > 0,
        selectAll: () => (selection = doc?.layers.map((l) => l.id) ?? []),
        align: (k) => void alignSelection(k),
        distribute: (a) => void distributeSelection(a),
        clearGuides: () => editGuides(NO_GUIDES),
        toggleLock: () => toggleLock(),
        editSafeAreas: () => (safePanel = true),
      }),
    );
      offCommands = () => {
        offEditor();
        offRename();
        offProvider();
      };
    });
    const off = () => {
      offFiles();
      offLive();
      offCommands();
      offStyles();
    };
    void (async () => {
      let st: ScreenState;
      try {
        [st] = await Promise.all([screensApi.getScreen(id), refreshSummaries()]);
      } catch (e) {
        if (alive) loadError = toAppError(e).message;
        return;
      }
      if (!alive) return;
      applyDoc(st);
      loaded = true;
      if (hasBindings(st.doc)) void loadSources();
      await ensureTemplates(tids(st.doc.layers)).catch(() => {});
    })();

    return () => {
      alive = false;
      off();
      rectSender.flush();
      if (deleted) autosave.cancel();
      else void autosave.flush();
    };
  });

  const savedTitle = $derived(updatedAt ? `Saved ${new Date(updatedAt).toLocaleString()}` : undefined);
  const previewStatus = $derived(statusText(status) ?? "The output server isn't running.");
</script>

<svelte:window {onkeydown} />
<svelte:head><title>{doc ? `${doc.name} · ` : ""}Screens · Kinetra</title></svelte:head>

<div class="frame">
  <div class="editor" class:banner={!!disk}>
    <header class="bar">
      <Button variant="ghost" size="sm" href="/screens">
        {#snippet icon()}<ArrowLeft size={14} />{/snippet}
        Screens
      </Button>
      <span class="sep" aria-hidden="true"></span>

      {#if loaded && doc}
        <span class="name vf vf-within">
          <input
            class="name-input"
            type="text"
            aria-label="Screen name"
            autocomplete="off"
            maxlength={SCREEN_NAME_MAX + 20}
            bind:value={nameDraft}
            aria-invalid={nameError ? "true" : undefined}
            aria-describedby={nameError ? `${uid}-name-err` : undefined}
            onkeydown={onNameKey}
            onblur={() => void commitName()}
          />
        </span>
        {#if nameError}<span class="bad small" id="{uid}-name-err">{nameError}</span>{/if}

        <span class="mono muted small" aria-label="Resolution {doc.width} by {doc.height}">{doc.width}×{doc.height}</span>

        <span class="bg">
          <Select
            size="sm"
            aria-label="Background"
            value={bgMode}
            options={BG_OPTIONS}
            onchange={(m: "transparent" | "colour") => setBgMode(m)}
          />
          {#if bgMode === "colour"}
            <span class="swatch vf vf-within">
              <input
                type="color"
                aria-label="Background color picker"
                value={pickerHex(doc.background)}
                oninput={(e) => setBackground(e.currentTarget.value)}
              />
            </span>
            <span class="hex vf vf-within">
              <input
                class="mono"
                type="text"
                aria-label="Background color"
                spellcheck="false"
                autocomplete="off"
                maxlength="9"
                value={bgDraft}
                aria-invalid={isBackground(doc.background) ? undefined : "true"}
                oninput={(e) => setBackground(e.currentTarget.value.trim())}
              />
            </span>
          {/if}
        </span>

        {#if shownUrl}
          <span class="url">
            <Link size={12} aria-hidden="true" />
            <span class="mono small" title={shownUrl}>{shownUrl}</span>
            <IconButton size="sm" label="Copy screen URL" onclick={() => void copyUrl(shownUrl)}>
              <Copy size={14} />
            </IconButton>
          </span>
        {/if}

        <span class="status small" role="status">
          {#if disk === "removed"}
            <span class="bad">Deleted on disk</span>
          {:else if saveBlocked}
            <span class="bad">Not saved: {saveBlocked}</span>
          {:else if saveState.kind === "failed"}
            <span class="bad">Not saved: {saveState.message}</span>
            <Button variant="ghost" size="sm" onclick={saveNow}>
              {#snippet icon()}<RotateCw size={12} />{/snippet}
              Retry
            </Button>
          {:else if saveState.kind === "saving" || saveState.kind === "pending"}
            <span class="muted">Saving…</span>
          {:else}
            <span class="muted" title={savedTitle}>Saved</span>
          {/if}
        </span>

        <span class="actions">
          <Button size="sm" loading={dupBusy} onclick={duplicate}>
            {#snippet icon()}<Copy size={14} />{/snippet}
            Duplicate
          </Button>
          <Button size="sm" variant="danger" onclick={askDelete}>
            {#snippet icon()}<Trash2 size={14} />{/snippet}
            Delete
          </Button>
        </span>
      {/if}
    </header>

    {#if disk}
      <!-- Rare: loaded only when a file changed on disk. -->
      {#await import("$lib/components/templates/editor/DiskBanner.svelte") then { default: DiskBanner }}
        <DiskBanner
        what="screen"
        state={disk}
        busy={diskBusy}
        onreload={diskReload}
        onkeep={diskKeep}
        onclose={() => void goto("/screens")}
        />
      {/await}
    {/if}

    {#if loadError}
      <div class="load-error">
        <p class="bad">Couldn't open this screen: {loadError}</p>
        <Button href="/screens">Back to screens</Button>
      </div>
    {:else if loaded && doc}
      <div class="body">
        <aside class="left" aria-label="Layers">
          {#await layersChunk then { default: LayersPanel }}
            <LayersPanel
              screenId={id}
              {rows}
              selectedIds={selection}
              templates={summaries}
              canAdd={canAddLayer(doc)}
              busyIds={playBusy}
              onselect={selectRow}
              onshift={shift}
              onreorder={reorder}
              ondelete={(l) => deleteLayers([l])}
              onplay={play}
              onadd={(t) => void addFromTemplate(t)}
              onnewtemplate={() => showGallery(null)}
              onmedia={() => void addMedia()}
            />
          {/await}
        </aside>

        <section class="centre" aria-label="Canvas">
          <div class="tools">
            {#await toolbarChunk then { default: Toolbar }}
              <Toolbar
                {view}
                onview={setView}
                guideCount={guides.x.length + guides.y.length}
                onclearguides={() => editGuides(NO_GUIDES)}
                safePanelOpen={safePanel}
                onsafepanel={() => (safePanel = !safePanel)}
              />
            {/await}
          </div>
          <div class="canvas">
            <Canvas
              screen={doc}
              layers={doc.layers}
              selectedIds={selection}
              src={localUrl}
              status={previewStatus}
              {view}
              {guides}
              safeAreas={doc.safeAreas ?? []}
              lockedIds={locked}
              designOf={(t) => templates.get(t)?.design}
              onresizestart={onResizeStart}
              onselect={(ids) => (selection = ids)}
              ondrag={onDrag}
              ondrop={commitRects}
              ondelete={deleteLayers}
              onguides={editGuides}
            />
            {#if safePanel}
              {#await loadSafePanel() then { default: SafeAreasPanel }}
                <SafeAreasPanel
                  areas={doc.safeAreas ?? []}
                  screen={doc}
                  onchange={editSafeAreas}
                  onclose={() => (safePanel = false)}
                />
              {/await}
            {/if}
          </div>
        </section>

        <aside class="right" aria-label="Inspector">
          {#if selected}
            {#await inspectorChunk then { default: Inspector }}
            <Inspector
              layer={selected}
              template={selected.kind !== "media" ? (templates.get(selected.templateId) ?? null) : null}
              missing={isMissing(selected)}
              templates={summaries}
              screen={doc}
              onname={(name) => patchSelected({ name })}
              ontemplate={(t) => void changeTemplate(t)}
              onvariant={(v) => void switchVariant(selected.id, v)}
              onstyles={() =>
                selected.kind !== "media" &&
                showGallery({ screenId: id, layerId: selected.id, name: selected.name, templateId: selected.templateId })}
              variantMissing={warnings.some((w) => w.code === "variantMissing" && w.layerId === selected.id)}
              onrect={(r, resized) => commitRects([{ id: selected.id, rect: r }], resized)}
              onsizing={(s) => setSizing([selected.id], s)}
              onopacity={(opacity) => patchSelected({ opacity })}
              onfit={(rect) => fitBoxes([{ id: selected.id, rect }])}
              aspectLocked={locked.has(selected.id)}
              onlockaspect={() => toggleLock(selected.id)}
              onalign={(k) => void alignSelection(k)}
              onanchor={(a) => anchorLayers([selected.id], a)}
              onfield={setField}
              onvar={setVar}
              onresetvar={resetVar}
              {sources}
              sourceWarnings={sourceWarnings.get(selected.id) ?? []}
              onloadsources={() => void loadSources()}
              onsourcesaved={() => void loadSources(true)}
              onbind={bind}
              onunbind={unbind}
              ontimer={(k, a) => void timer(k, a)}
              screenId={id}
              live={liveLayers[selected.id]}
              onpatch={patchSelected}
            />
            {/await}
          {:else if selectedLayers.length > 1}
            {#await loadMulti() then { default: MultiInspector }}
              <MultiInspector
                layers={selectedLayers}
                onmove={moveSelection}
                onsizing={(s) => setSizing(selection, s)}
                {templates}
                screen={doc}
                onfit={fitBoxes}
                onalign={(k) => void alignSelection(k)}
                ondistribute={(a) => void distributeSelection(a)}
                onanchor={(a) => anchorLayers(selection, a)}
              />
            {/await}
          {:else}
            <p class="hint">
              {doc.layers.length === 0
                ? "Add a layer to place a template or media on this screen."
                : "Select a layer on the canvas or in the list to edit it."}
            </p>
          {/if}
        </aside>
      </div>
    {/if}
  </div>
</div>

{#if Confirm && confirmOpen && doc}
  <Confirm
    title="Delete screen?"
    message="“{doc.name}” will be deleted from this project and its URL stops working. Backups keep a copy."
    confirmLabel="Delete"
    danger
    busy={deleteBusy}
    onconfirm={confirmDelete}
    oncancel={() => (confirmOpen = false)}
  />
{/if}

{#if gallery.component}
  <gallery.component open={gallery.open} layer={styleLayer} onclose={gallery.close} />
{/if}

<style>
  .frame {
    container-type: inline-size;
  }
  .editor {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    height: calc(100dvh - var(--topbar-h));
    min-width: 0;
  }
  /* The DiskBanner row sits between the header and the body only while it shows. */
  .editor.banner {
    grid-template-rows: auto auto minmax(0, 1fr);
  }

  /* ---- Header ---- */
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
    min-height: calc(var(--row-h) + var(--s-4));
    padding: var(--s-2) var(--s-4);
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .sep {
    width: 1px;
    height: var(--s-4);
    background: var(--line);
  }
  .name {
    display: inline-flex;
    min-width: 0;
    flex: 0 1 22rem;
    border-radius: var(--r-control);
  }
  .name-input {
    width: 100%;
    min-width: 8rem;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    font-family: var(--font-heading);
    font-size: var(--fs-xl);
    font-weight: 600;
  }
  .name-input:hover,
  .name-input:focus {
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .name-input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .bg {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
  }
  .swatch,
  .hex {
    display: inline-flex;
    border-radius: var(--r-control);
  }
  .swatch input {
    width: var(--control-h-sm);
    height: var(--control-h-sm);
    padding: 2px;
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    cursor: pointer;
  }
  .hex input {
    width: calc(9ch + 2 * var(--s-2));
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    font-size: var(--fs-sm);
  }
  .hex input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .url {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
    color: var(--text-muted);
  }
  .url .mono {
    overflow: hidden;
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    margin-left: auto;
  }
  .actions {
    display: inline-flex;
    gap: var(--s-2);
  }
  .small {
    font-size: var(--fs-sm);
  }
  .muted {
    color: var(--text-muted);
  }
  .bad {
    color: var(--danger);
  }
  .load-error {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-3);
    padding: var(--s-6);
  }

  /* ---- Body ---- */
  .body {
    display: grid;
    grid-template-columns: minmax(16rem, 20rem) minmax(0, 1fr) minmax(18rem, 24rem);
    min-height: 0;
  }
  .left,
  .right {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: auto;
  }
  .left {
    border-right: 1px solid var(--line);
  }
  .right {
    border-left: 1px solid var(--line);
    background: var(--surface-1);
  }
  .centre {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    min-width: 0;
    min-height: 0;
  }
  /* The toolbar's row is reserved before its chunk arrives, so the canvas never jumps. */
  .tools {
    min-height: calc(var(--row-h) + 1px);
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .canvas {
    position: relative;
    display: grid;
    min-width: 0;
    min-height: 0;
  }
  .hint {
    padding: var(--s-4);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }

  @container (max-width: 1100px) {
    .editor {
      height: auto;
    }
    .body {
      grid-template-columns: minmax(14rem, 18rem) minmax(0, 1fr);
    }
    .centre {
      min-height: 60vh;
    }
    .right {
      grid-column: 1 / -1;
      border-left: 0;
      border-top: 1px solid var(--line);
      overflow: visible;
    }
  }
</style>
