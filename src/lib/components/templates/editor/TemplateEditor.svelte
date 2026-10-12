<!--
  TemplateEditor: the /templates/[id] page body. Header (name, save status, Duplicate, Delete), a
  tabbed editor on the left (HTML | CSS | Fields | Variables) and the live preview on the right.
  The preview opens with the graphic on air and held (no auto-out), so every value, image or
  look the user sets shows at once; Play out / Play in replay it as on air, auto-out included (the
  auto-out setting is saved but only Play in arms it in the preview).
  Simple mode: Fields | Variables only, with the "Available in Advanced mode" hint for the code.

  Props:
    id: string   the template id (remount the component to switch templates)

  Draft + autosave:
  - Every edit updates local state at once. The preview gets `previewDraft` with only the changed
    part, debounced 150 ms per kind (DraftSender). HTML/CSS are read from CodeMirror only when a
    draft or a save is actually sent, never per keystroke.
  - Autosave 800 ms after the last edit (Autosave): one save in flight, the latest doc queued; never
    while there are validation errors (the status says why). Ctrl/Cmd+S saves at once. Unmount
    flushes a pending save (fired, not awaited), drops pending drafts and calls previewHide().
  - CodeMirror loads lazily (CodeEditor → dynamic import of ./codemirror), one view per code tab,
    created on the tab's first visit and destroyed on unmount.

  Variants (VariantTabs above the editor): the field schema and variable definitions are shared
  and saved with `saveTemplate` (meta); each variant's HTML, CSS and variable values are saved
  with `saveTemplateVariant`, one coalesced Autosave per variant (a variant save first flushes
  the meta save, so new variable keys exist on disk). Switching variants keeps the text of the
  one left in its working copy, recreates the code views with the new one's text (memory stays
  flat: never more than one view per code tab) and points the preview at it (`variant` draft).
  The Variables tab's values are the active variant's `vars` (missing keys show the definition
  default).

  Newer look: when the preset this template was made from ships a newer version (the summary's
  `presetUpdate`), the header shows "A newer look is available · Update" (UpdateNotice, lazy) and
  the palette gets "Update {template} to the latest look". Both save first and reload after the
  update or its undo.

  Changed on disk (`project-files-changed` for this template): without unsaved edits it reloads
  silently (toast); with unsaved edits a DiskBanner offers Reload / Keep mine and autosave waits
  for the choice. Deleted on disk: a banner, Close goes back to the list. Invalid: a toast, the
  working copy stays.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import ArrowLeft from "lucide-svelte/icons/arrow-left";
  import Copy from "lucide-svelte/icons/copy";
  import RotateCw from "lucide-svelte/icons/rotate-cw";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import { goto } from "$app/navigation";
  import { on, toAppError } from "$lib/api";
  import * as api from "$lib/api/templates";
  import { registerCommands } from "$lib/commands/registry.svelte";
  import Button from "$lib/components/Button.svelte";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import {
    TEMPLATE_CSS_MAX,
    TEMPLATE_HTML_MAX,
    type FieldDef,
    type FilesChanged,
    type PresetRef,
    type PresetUpdate,
    type TemplateDesign,
    type TemplateDoc,
    type VariableDef,
    type VariantDoc,
  } from "$lib/types";
  import type { EditorHandle } from "./codemirror";
  import CodeEditor from "./CodeEditor.svelte";
  import DiskBanner from "./DiskBanner.svelte";
  import PreviewPanel from "./PreviewPanel.svelte";
  import VariantTabs, { type VariantActions } from "./VariantTabs.svelte";
  import { hasRows, previewDefault, typeIssues } from "./fieldTypes";
  import { describeIssue, NAME_MAX, validate, valueIssue } from "./model";
  import { markRow, rotateMs, rotatingLists, usesCurrent } from "./rotate";
  import { Autosave, DraftSender, diskReaction, type SaveState } from "./sync";
  import { applyDefs, effectiveValues, effectiveVariables, pruneVars } from "./variants";

  let { id }: { id: string } = $props();

  const DRAFT_DEBOUNCE_MS = 150;
  const AUTOSAVE_MS = 800;

  const uid = $props.id();

  // ---- Document state ----------------------------------------------------------------------------

  let loaded = $state(false);
  let loadError = $state<string | null>(null);

  let name = $state("");
  let description = "";
  let preset: PresetRef | null = null;
  /** The design box: no UI edits it, but every save carries it through unchanged. */
  let design: TemplateDesign | null = null;
  let updatedAt = $state<string | null>(null);
  let fields = $state.raw<FieldDef[]>([]);
  let variables = $state.raw<VariableDef[]>([]);
  let autoOutMs = $state<number | null>(null);

  /**
   * Working copies of every variant. The active one's html/css live in the CodeMirror views once
   * created and are written back here on save and when switching away (`commitActive`).
   */
  let variantList = $state.raw<VariantDoc[]>([]);
  let defaultVariant = $state("");
  let active = $state("");
  const activeDoc = $derived(variantList.find((v) => v.id === active));
  /** The definitions with the active variant's values as `default` (what the Variables tab shows). */
  const shownVariables = $derived(effectiveVariables(variables, activeDoc?.vars ?? {}));

  let htmlView: EditorHandle | null = null;
  let cssView: EditorHandle | null = null;
  const readHtml = () => htmlView?.getValue() ?? activeDoc?.html ?? "";
  const readCss = () => cssView?.getValue() ?? activeDoc?.css ?? "";
  // Only flips at the size limit, so typing doesn't re-run validation.
  let htmlTooBig = $state(false);
  let cssTooBig = $state(false);

  function updateVariant(vid: string, patch: Partial<VariantDoc>) {
    variantList = variantList.map((v) => (v.id === vid ? { ...v, ...patch } : v));
  }

  /** Writes the code views' text into the active variant's working copy (only when it changed). */
  function commitActive() {
    const v = activeDoc;
    if (!v) return;
    const html = readHtml();
    const css = readCss();
    if (html !== v.html || css !== v.css) updateVariant(v.id, { html, css });
  }

  const issues = $derived(
    validate({
      fields,
      variables: shownVariables,
      htmlLength: htmlTooBig ? TEMPLATE_HTML_MAX + 1 : 0,
      cssLength: cssTooBig ? TEMPLATE_CSS_MAX + 1 : 0,
      fieldIssues: typeIssues,
    }),
  );
  const blockedReason = $derived(issues.length > 0 ? describeIssue(issues[0]) : null);
  /** Changed or deleted on disk while there were unsaved edits: the banner shows, saving waits. */
  let disk = $state<"conflict" | "removed" | null>(null);
  let diskBusy = $state(false);
  const saveBlocked = $derived(
    disk === "conflict" ? "Changed on disk: choose Reload or Keep mine." : blockedReason,
  );
  const issueCount = $derived({
    html: issues.filter((i) => i.scope === "html").length,
    css: issues.filter((i) => i.scope === "css").length,
    fields: issues.filter((i) => i.scope === "field").length,
    variables: issues.filter((i) => i.scope === "variable").length,
  });

  // Preview-only value per field: what the user typed, else the field's label (a list: its default
  // rows, else the preset's sample rows, else one row of its column labels; a clock: its format; a
  // timer: its default state).
  let typedPreview = $state<Record<string, string>>({});
  /** A preset template's sample rows for its list fields without default rows (loaded lazily). */
  let sampleLists = $state.raw<Record<string, string>>({});
  const previewValues = $derived(
    Object.fromEntries(
      fields.map((f) => [
        f.key,
        typedPreview[f.key] ??
          (f.type === "list" && !hasRows(f.default) ? sampleLists[f.key] : undefined) ??
          previewDefault(f),
      ]),
    ),
  );

  /** Loads the preset's sample rows for list fields without default rows: a slideshow needs real
   *  rows for its preview to show a slide change. */
  async function loadSampleLists(presetId: string) {
    const keys = fields.filter((f) => f.type === "list" && !hasRows(f.default)).map((f) => f.key);
    if (!keys.length) return;
    try {
      const { presetSample } = await import("../gallery/presets");
      const sample = await presetSample(presetId, active);
      const out: Record<string, string> = {};
      for (const k of keys) if (sample[k] && hasRows(sample[k])) out[k] = sample[k];
      sampleLists = out;
    } catch {
      // No sample data in this build: the column-label row stays.
    }
  }

  /** The shared meta (fields, variable definitions, variant list) as `saveTemplate` takes it. */
  function snapshot(): TemplateDoc {
    commitActive();
    return {
      id,
      name,
      description,
      preset,
      fields,
      variables,
      autoOutMs,
      ...(design ? { design } : {}),
      updatedAt: updatedAt ?? "",
      defaultVariant,
      variants: variantList.map((v) => ({ ...v, vars: pruneVars(variables, v.vars) })),
    };
  }

  // ---- Save + drafts -----------------------------------------------------------------------------

  /** Save state per autosave ("meta" or a variant id); the header shows the combined state. */
  let saveStates = $state<Record<string, SaveState>>({});
  const saveState = $derived.by((): SaveState => {
    const all = Object.values(saveStates);
    return (
      all.find((s) => s.kind === "failed") ??
      all.find((s) => s.kind === "saving" || s.kind === "pending") ?? { kind: "saved", at: updatedAt }
    );
  });

  function report(key: string, s: SaveState) {
    if (s.kind === "saved" && s.at) updatedAt = s.at;
    const prev = saveStates[key];
    if (prev?.kind === s.kind && s.kind !== "saved" && s.kind !== "failed") return; // no churn per keystroke
    saveStates[key] = s;
  }

  const describe = (e: unknown) => toAppError(e).message;

  /** Shared meta: name-independent parts of template.json. */
  const autosave = new Autosave<TemplateDoc>({
    delayMs: AUTOSAVE_MS,
    snapshot,
    blocked: () => saveBlocked,
    save: (doc) => api.saveTemplate(doc),
    onState: (s) => report("meta", s),
    describe,
  });

  /** One autosave per variant (html, css, vars), created on its first edit. */
  const variantSaves = new Map<string, Autosave<VariantDoc>>();

  function variantSave(vid: string): Autosave<VariantDoc> {
    let a = variantSaves.get(vid);
    if (a) return a;
    a = new Autosave<VariantDoc>({
      delayMs: AUTOSAVE_MS,
      snapshot: () => {
        if (vid === active) commitActive();
        const v = variantList.find((x) => x.id === vid);
        return v ? { ...v, vars: pruneVars(variables, v.vars) } : { id: vid, name: "", vars: {}, html: "", css: "" };
      },
      blocked: () => saveBlocked ?? (variantList.some((x) => x.id === vid) ? null : "deleted"),
      save: async (v) => {
        await autosave.flush(); // new variable keys must exist before a variant uses them
        return api.saveTemplateVariant(id, v.id, v.html, v.css, v.vars);
      },
      onState: (s) => report(vid, s),
      describe,
    });
    variantSaves.set(vid, a);
    return a;
  }

  const saves = () => [autosave, ...variantSaves.values()];
  const unsaved = () => saves().some((a) => a.dirty || a.saving);
  const flushAll = () => Promise.all(saves().map((a) => a.flush())).then(() => {});

  function dropVariantSave(vid: string) {
    variantSaves.get(vid)?.cancel();
    variantSaves.delete(vid);
    delete saveStates[vid];
  }

  const drafts = new DraftSender(DRAFT_DEBOUNCE_MS, (d) => api.previewDraft(d));

  function onHtmlChange() {
    if (htmlView) htmlTooBig = htmlView.length() > TEMPLATE_HTML_MAX;
    drafts.queue("html", readHtml);
    variantSave(active).touch();
  }

  function onCssChange() {
    if (cssView) cssTooBig = cssView.length() > TEMPLATE_CSS_MAX;
    drafts.queue("css", readCss);
    variantSave(active).touch();
  }

  /** Shows variant `vid` in the editors and the preview. */
  function selectVariant(vid: string) {
    const next = variantList.find((v) => v.id === vid);
    if (!next || vid === active) return;
    commitActive();
    void drafts.flush(); // the variant left gets its last edits (sent now, in order)
    htmlView = null;
    cssView = null;
    active = vid;
    htmlTooBig = next.html.length > TEMPLATE_HTML_MAX;
    cssTooBig = next.css.length > TEMPLATE_CSS_MAX;
    api
      .previewDraft({
        variant: vid,
        html: next.html,
        css: next.css,
        variableValues: effectiveValues(variables, next.vars),
      })
      .catch(fail);
  }

  function setFields(next: FieldDef[]) {
    // A field whose type changed drops its typed preview value (it no longer fits).
    const before = new Map(fields.map((f) => [f.key, f.type]));
    for (const f of next) {
      const t = before.get(f.key);
      if (t !== undefined && t !== f.type && f.key in typedPreview) delete typedPreview[f.key];
    }
    fields = next;
    if (!issues.some((i) => i.scope === "field")) {
      drafts.queue("fields", next);
      drafts.merge("fieldValues", previewValues);
    }
    autosave.touch();
  }

  function setVariables(next: VariableDef[]) {
    const r = applyDefs(variables, shownVariables, next, variantList, active);
    variables = r.defs;
    variantList = r.variants;
    // The preview shows the active variant's values as the definitions' defaults.
    if (!issues.some((i) => i.scope === "variable")) drafts.queue("variables", () => shownVariables);
    autosave.touch();
    for (const vid of r.changed) variantSave(vid).touch();
  }

  function setVariableValue(index: number, value: string) {
    const v = { ...shownVariables[index], default: value } as VariableDef;
    if (!activeDoc) return;
    updateVariant(active, { vars: { ...activeDoc.vars, [v.key]: value } });
    if (valueIssue(v) === null) drafts.merge("variableValues", { [v.key]: value });
    variantSave(active).touch();
  }

  function setPreviewValue(key: string, value: string) {
    typedPreview[key] = value;
    drafts.merge("fieldValues", { [key]: value });
  }

  function setAutoOut(ms: number | null) {
    autoOutMs = ms;
    // Saved, but not sent to the preview: a held preview would otherwise play out by itself.
    // Play in arms it.
    autosave.touch();
  }

  function saveNow() {
    if (!loaded) return;
    void autosave.saveNow();
    void variantSave(active).saveNow();
    for (const [vid, a] of variantSaves) if (vid !== active) void a.flush();
  }

  function onkeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "s") {
      e.preventDefault();
      saveNow();
    }
  }

  // ---- Load / teardown ---------------------------------------------------------------------------

  let deleted = false;

  function fail(e: unknown) {
    toasts.push({ kind: "error", message: toAppError(e).message });
  }

  /** Bumped on a reload from disk: remounts the code views with the new text. */
  let loadGen = $state(0);

  function applyDoc(doc: TemplateDoc) {
    name = doc.name;
    nameDraft = doc.name;
    nameError = null;
    description = doc.description;
    preset = doc.preset;
    design = doc.design ?? null;
    updatedAt = doc.updatedAt;
    fields = doc.fields;
    variables = doc.variables;
    autoOutMs = doc.autoOutMs;
    variantList = doc.variants;
    defaultVariant = doc.defaultVariant;
    // Stay on the variant being edited when it still exists.
    if (!doc.variants.some((v) => v.id === active))
      active = doc.variants.some((v) => v.id === doc.defaultVariant) ? doc.defaultVariant : (doc.variants[0]?.id ?? "");
    const v = doc.variants.find((x) => x.id === active);
    htmlTooBig = (v?.html.length ?? 0) > TEMPLATE_HTML_MAX;
    cssTooBig = (v?.css.length ?? 0) > TEMPLATE_CSS_MAX;
    saveStates = {};
    checkUpdate();
  }

  // ---- Newer look from the preset --------------------------------------------------------------

  /** The preset this template was made from ships a newer look (read from the cached list). */
  let presetUpdate = $state.raw<PresetUpdate | null>(null);

  function checkUpdate() {
    if (!preset) {
      presetUpdate = null;
      return;
    }
    api.listTemplates().then(
      (list) => (presetUpdate = list.find((t) => t.id === id)?.presetUpdate ?? null),
      () => {},
    );
  }

  /** Saves first; the update (or its undo) reloads the working copy from disk. */
  const updateHooks = { before: () => flushAll(), after: () => void reloadFromDisk() };

  async function runUpdate() {
    const { updateLook } = await import("../update");
    await updateLook({ id, name }, updateHooks);
  }

  // The palette command for this template (the project-items one is hidden in the editor).
  $effect(() => {
    if (!presetUpdate) return;
    return registerCommands([
      {
        id: `template.update:${id}`,
        title: "Update {template} to the latest look",
        params: () => ({ template: name }),
        section: "edit",
        scope: "template",
        run: runUpdate,
      },
    ]);
  });

  /** Points the preview at the active variant when it isn't the default (which previewTemplate shows). */
  function previewActive() {
    const v = activeDoc;
    if (!v || v.id === defaultVariant) return Promise.resolve();
    return api.previewDraft({
      variant: v.id,
      html: v.html,
      css: v.css,
      variableValues: effectiveValues(variables, v.vars),
    });
  }

  /** Replaces the working copy with the file on disk and reloads the preview. */
  async function reloadFromDisk(): Promise<boolean> {
    let doc: TemplateDoc;
    try {
      doc = await api.getTemplate(id);
    } catch (e) {
      fail(e);
      return false;
    }
    for (const a of saves()) a.discard();
    drafts.cancel();
    htmlView = null;
    cssView = null;
    applyDoc(doc);
    loadGen++;
    disk = null;
    try {
      await api.previewTemplate(id);
      await previewActive();
      if (fields.length > 0) await api.previewDraft({ fieldValues: { ...previewValues } });
      if (onAir) await showHeld();
    } catch (e) {
      fail(e);
    }
    return true;
  }

  function onFilesChanged(e: FilesChanged) {
    if (!loaded || deleted) return;
    const reaction = diskReaction(e, "template", id, unsaved() || disk === "conflict");
    switch (reaction) {
      case "reload":
        void reloadFromDisk().then((ok) => {
          if (ok) toasts.push({ kind: "info", message: `“${name}” changed on disk and was reloaded.` });
        });
        break;
      case "conflict":
        disk = "conflict";
        break;
      case "removed":
        deleted = true;
        for (const a of saves()) a.cancel();
        disk = "removed";
        break;
      case "invalid":
        toasts.push({
          kind: "error",
          message: `“${name}” changed on disk but couldn't be loaded: ${e.message ?? "invalid file"}. Your copy here is kept.`,
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

  /** Keep mine: writes the whole working copy (meta and every variant) over the disk version. */
  async function diskKeep() {
    disk = null;
    diskBusy = true;
    try {
      await Promise.all([autosave.saveNow(), ...variantList.map((v) => variantSave(v.id).saveNow())]);
    } finally {
      diskBusy = false;
    }
  }

  onMount(() => {
    let alive = true;
    const off = on("project-files-changed", onFilesChanged);
    void (async () => {
      let doc: TemplateDoc;
      try {
        doc = await api.getTemplate(id);
      } catch (e) {
        if (alive) loadError = toAppError(e).message;
        return;
      }
      if (!alive) return;
      applyDoc(doc);
      loaded = true;
      const samples = doc.preset ? loadSampleLists(doc.preset.id) : null;
      try {
        const layer = await api.previewTemplate(id);
        if (!alive) return;
        onAir = layer.onAir;
        await previewActive();
        await samples;
        if (!alive) return;
        // Preview text: each field's label until the user types something else.
        if (fields.length > 0) await api.previewDraft({ fieldValues: { ...previewValues } });
        if (!alive) return;
        // Show the graphic right away, so a picked image or a typed value is visible at once.
        await showHeld();
      } catch (e) {
        if (alive) fail(e);
      }
    })();

    return () => {
      alive = false;
      off();
      drafts.cancel();
      clearAutoOutTimer();
      for (const a of saves()) {
        if (deleted) a.cancel();
        else void a.flush();
      }
      api.previewHide().catch(() => {});
    };
  });

  // ---- Variant actions (VariantTabs' menu) -------------------------------------------------------

  /** Runs a variant command after flushing every save; merges the result. False on failure. */
  async function variantOp(run: () => Promise<TemplateDoc>, merge: (doc: TemplateDoc) => void): Promise<boolean> {
    try {
      await flushAll();
      const doc = await run();
      updatedAt = doc.updatedAt;
      merge(doc);
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  }

  const variantActions: VariantActions = {
    duplicate: (vname) =>
      variantOp(
        () => api.createTemplateVariant(id, active, vname),
        (doc) => {
          const known = new Set(variantList.map((v) => v.id));
          const added = doc.variants.filter((v) => !known.has(v.id));
          variantList = [...variantList, ...added];
          if (added[0]) {
            selectVariant(added[0].id);
            toasts.push({ kind: "success", message: `Added variant “${added[0].name}”` });
          }
        },
      ),
    rename: (vid, vname) =>
      variantOp(
        () => api.renameTemplateVariant(id, vid, vname),
        (doc) => updateVariant(vid, { name: doc.variants.find((v) => v.id === vid)?.name ?? vname }),
      ),
    remove: (vid) => {
      dropVariantSave(vid);
      const gone = variantList.find((v) => v.id === vid)?.name ?? "";
      return variantOp(
        () => api.deleteTemplateVariant(id, vid),
        (doc) => {
          defaultVariant = doc.defaultVariant;
          if (active === vid) {
            htmlView = null;
            cssView = null;
            const next = doc.variants.some((v) => v.id === doc.defaultVariant) ? doc.defaultVariant : doc.variants[0]?.id;
            variantList = variantList.filter((v) => v.id !== vid);
            active = "";
            if (next) selectVariant(next);
          } else variantList = variantList.filter((v) => v.id !== vid);
          toasts.push({ kind: "success", message: `Deleted variant “${gone}”` });
        },
      );
    },
    setDefault: (vid) =>
      variantOp(
        () => api.setDefaultTemplateVariant(id, vid),
        (doc) => (defaultVariant = doc.defaultVariant),
      ),
  };

  // ---- Play in / out -----------------------------------------------------------------------------

  let onAir = $state(false);
  let playBusy = $state(false);
  let autoOutTimer: ReturnType<typeof setTimeout> | undefined;

  function clearAutoOutTimer() {
    if (autoOutTimer !== undefined) clearTimeout(autoOutTimer);
    autoOutTimer = undefined;
  }

  /**
   * Puts the graphic on air in the preview without auto-out (the editor opens this way, so what
   * the user sets is visible at once). Play in plays it for real, auto-out included.
   */
  async function showHeld() {
    clearAutoOutTimer();
    await api.previewDraft({ autoOutMs: null });
    const layer = await api.previewShow();
    onAir = layer.onAir;
  }

  async function playIn() {
    if (playBusy) return;
    playBusy = true;
    onAir = true; // optimistic
    try {
      await drafts.flush(); // play in what is on screen right now
      await api.previewDraft({ autoOutMs });
      const layer = await api.previewShow();
      onAir = layer.onAir;
      clearAutoOutTimer();
      // The server plays it out after autoOutMs; mirror that here (one timer, only while on air).
      if (onAir && autoOutMs !== null) {
        autoOutTimer = setTimeout(() => {
          autoOutTimer = undefined;
          onAir = false;
        }, autoOutMs);
      }
    } catch (e) {
      onAir = false;
      fail(e);
    } finally {
      playBusy = false;
    }
  }

  async function playOut() {
    if (playBusy) return;
    playBusy = true;
    onAir = false; // optimistic
    clearAutoOutTimer();
    try {
      const layer = await api.previewHide();
      onAir = layer.onAir;
    } catch (e) {
      onAir = true;
      fail(e);
    } finally {
      playBusy = false;
    }
  }

  // ---- Slide rotation (./rotate.ts) --------------------------------------------------------------

  /** List keys whose `_current` mark the preview moves: on air, CSS styling data-current, 2+ rows. */
  const rotating = $derived(
    onAir && loaded && usesCurrent(activeDoc?.css ?? "") ? rotatingLists(fields, previewValues) : [],
  );
  const rotateEvery = $derived(rotateMs(shownVariables));

  // One interval, only while something rotates on air; the values are read when a step fires.
  $effect(() => {
    const keys = rotating;
    const ms = rotateEvery;
    if (!keys.length) return;
    let step = 0;
    const timer = setInterval(() => {
      if (document.hidden) return;
      step++;
      const v = untrack(() => previewValues);
      drafts.merge("fieldValues", Object.fromEntries(keys.map((k) => [k, markRow(v[k] ?? "", step)])));
    }, ms);
    return () => clearInterval(timer);
  });

  // ---- Name --------------------------------------------------------------------------------------

  let nameDraft = $state("");
  let nameError = $state<string | null>(null);

  async function commitName() {
    const next = nameDraft.trim();
    if (next === name) {
      nameDraft = next;
      nameError = null;
      return;
    }
    if (next === "") {
      nameError = "Enter a name.";
      return;
    }
    if (next.length > NAME_MAX) {
      nameError = `Up to ${NAME_MAX} characters.`;
      return;
    }
    nameError = null;
    const prev = name;
    name = next; // optimistic
    try {
      const summary = await api.renameTemplate(id, next);
      name = summary.name;
      nameDraft = summary.name;
      updatedAt = summary.updatedAt;
      // A save that started before the rename carries the old name: save once more.
      if (autosave.saving) autosave.touch();
    } catch (e) {
      name = prev;
      nameDraft = prev;
      fail(e);
    }
  }

  function onNameKey(e: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (e.key === "Enter") {
      e.preventDefault();
      void commitName();
    } else if (e.key === "Escape") {
      nameDraft = name;
      nameError = null;
      e.currentTarget.blur();
    }
  }

  // ---- Duplicate / delete ------------------------------------------------------------------------

  let dupBusy = $state(false);

  async function duplicate() {
    if (dupBusy) return;
    dupBusy = true;
    try {
      await flushAll(); // the copy is made from what is saved
      const copy = await api.duplicateTemplate(id, `${name} copy`.slice(0, NAME_MAX));
      toasts.push({ kind: "success", message: `Duplicated as “${copy.name}”` });
      await goto(`/templates/${copy.id}`);
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
      for (const a of saves()) a.cancel();
      deleted = true;
      await api.deleteTemplate(id);
      confirmOpen = false;
      toasts.push({ kind: "success", message: `Deleted “${name}”` });
      await goto("/templates");
    } catch (e) {
      deleted = false;
      fail(e);
    } finally {
      deleteBusy = false;
    }
  }

  // ---- Tabs --------------------------------------------------------------------------------------

  const ALL_TABS = [
    { id: "html", label: "HTML" },
    { id: "css", label: "CSS" },
    { id: "fields", label: "Fields" },
    { id: "variables", label: "Variables" },
  ] as const;
  type TabId = (typeof ALL_TABS)[number]["id"];
  /** Simple mode hides the code (HTML / CSS): fields, variables and the preview stay. */
  const advanced = $derived(isAdvanced());
  const TABS = $derived(advanced ? ALL_TABS : ALL_TABS.filter((x) => x.id !== "html" && x.id !== "css"));

  let tab = $state<TabId>(untrack(() => (isAdvanced() ? "html" : "fields")));
  /** Code views are created on the tab's first visit and kept while the page is open. */
  let visited = $state({ html: true, css: false });

  // Switching to Simple: keep what the code views hold (they unmount), then show Fields.
  $effect.pre(() => {
    if (advanced) return;
    untrack(() => {
      commitActive();
      htmlView = null;
      cssView = null;
      if (tab === "html" || tab === "css") tab = "fields";
    });
  });
  let tabButtons: HTMLButtonElement[] = $state([]);

  function select(t: TabId) {
    tab = t;
    if (t === "css") visited.css = true;
    if (t === "html") visited.html = true;
  }

  function onTabKey(e: KeyboardEvent, i: number) {
    let next = -1;
    if (e.key === "ArrowRight") next = (i + 1) % TABS.length;
    else if (e.key === "ArrowLeft") next = (i - 1 + TABS.length) % TABS.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = TABS.length - 1;
    if (next < 0) return;
    e.preventDefault();
    select(TABS[next].id);
    tabButtons[next]?.focus();
  }

  const fieldCompletions = () => fields.map((f) => ({ key: f.key, label: f.label }));
  const variableCompletions = () => variables.map((v) => ({ key: v.key, label: v.label }));
  const fieldKeys = $derived(fields.map((f) => f.key));
  const variableKeys = $derived(variables.map((v) => v.key));

  const savedTitle = $derived(updatedAt ? `Saved ${new Date(updatedAt).toLocaleString()}` : undefined);
</script>

<svelte:window {onkeydown} />
<svelte:head><title>{name ? `${name} · ` : ""}Templates · Kinetra</title></svelte:head>

<div class="frame">
  <div class="editor" class:banner={!!disk}>
    <header class="bar">
      <Button variant="ghost" size="sm" href="/templates">
        {#snippet icon()}<ArrowLeft size={14} />{/snippet}
        Templates
      </Button>
      <span class="sep" aria-hidden="true"></span>

      {#if loaded}
        <span class="name vf vf-within">
          <input
            class="name-input"
            type="text"
            aria-label="Template name"
            autocomplete="off"
            maxlength={NAME_MAX + 20}
            bind:value={nameDraft}
            aria-invalid={nameError ? "true" : undefined}
            aria-describedby={nameError ? `${uid}-name-err` : undefined}
            onkeydown={onNameKey}
            onblur={() => void commitName()}
          />
        </span>
        {#if nameError}<span class="bad small" id="{uid}-name-err">{nameError}</span>{/if}

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

        {#if presetUpdate}
          <!-- Only for an outdated preset template: its own small chunk. -->
          {#await import("../UpdateNotice.svelte") then { default: UpdateNotice }}
            <UpdateNotice target={{ id, name }} {...updateHooks} />
          {/await}
        {/if}

        <span class="actions">
          {#await import("$lib/components/ai/AiEntry.svelte") then { default: AiEntry }}
            <AiEntry mode={{ kind: "tweak", templateId: id, name }} />
          {/await}
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
      <DiskBanner
        what="template"
        state={disk}
        busy={diskBusy}
        onreload={diskReload}
        onkeep={diskKeep}
        onclose={() => void goto("/templates")}
      />
    {/if}

    {#if loadError}
      <div class="load-error">
        <p class="bad">Couldn't open this template: {loadError}</p>
        <Button href="/templates">Back to templates</Button>
      </div>
    {:else if loaded}
      <div class="body">
        <div class="left">
          <VariantTabs
            variants={variantList}
            {active}
            {defaultVariant}
            {fields}
            {variables}
            panelId="{uid}-variant"
            onselect={selectVariant}
            actions={variantActions}
          />
          <div class="vpanel" role="tabpanel" id="{uid}-variant" aria-label="Variant {activeDoc?.name ?? ''}">
          <div class="tabs" role="tablist" aria-label="Template parts">
            {#each TABS as t, i (t.id)}
              <button
                bind:this={tabButtons[i]}
                type="button"
                role="tab"
                class="tab"
                id="{uid}-tab-{t.id}"
                aria-selected={tab === t.id}
                aria-controls="{uid}-panel-{t.id}"
                tabindex={tab === t.id ? 0 : -1}
                onclick={() => select(t.id)}
                onkeydown={(e) => onTabKey(e, i)}
              >
                {t.label}
                {#if issueCount[t.id] > 0}
                  <span class="tab-issues" aria-label="{issueCount[t.id]} to fix">{issueCount[t.id]}</span>
                {/if}
              </button>
            {/each}
          </div>

          {#if !advanced}
            {#await import("$lib/components/AdvancedHint.svelte") then { default: AdvancedHint }}
              <div class="code-hint"><AdvancedHint feature="HTML and CSS" /></div>
            {/await}
          {/if}
          {#if advanced}
          <div
            class="panel code"
            role="tabpanel"
            id="{uid}-panel-html"
            aria-labelledby="{uid}-tab-html"
            hidden={tab !== "html"}
          >
            {#if visited.html}
              {#key `${loadGen}:${active}`}
              <CodeEditor
                lang="html"
                doc={activeDoc?.html ?? ""}
                label="Template HTML"
                onchange={onHtmlChange}
                onready={(h) => (htmlView = h)}
                completions={fieldCompletions}
              />
              {/key}
            {/if}
          </div>
          <div
            class="panel code"
            role="tabpanel"
            id="{uid}-panel-css"
            aria-labelledby="{uid}-tab-css"
            hidden={tab !== "css"}
          >
            {#if visited.css}
              {#key `${loadGen}:${active}`}
              <CodeEditor
                lang="css"
                doc={activeDoc?.css ?? ""}
                label="Template CSS"
                onchange={onCssChange}
                onready={(h) => (cssView = h)}
                completions={variableCompletions}
              />
              {/key}
            {/if}
          </div>
          {/if}
          <div
            class="panel"
            role="tabpanel"
            id="{uid}-panel-fields"
            aria-labelledby="{uid}-tab-fields"
            hidden={tab !== "fields"}
          >
            {#if tab === "fields"}
              {#await import("./FieldsForm.svelte") then { default: FieldsForm }}
                <FieldsForm {fields} otherKeys={variableKeys} {issues} onchange={setFields} />
              {/await}
            {/if}
          </div>
          <div
            class="panel"
            role="tabpanel"
            id="{uid}-panel-variables"
            aria-labelledby="{uid}-tab-variables"
            hidden={tab !== "variables"}
          >
            {#if tab === "variables"}
              {#await import("./VariablesForm.svelte") then { default: VariablesForm }}
                <VariablesForm
                  variables={shownVariables}
                  otherKeys={fieldKeys}
                  {issues}
                  onvalue={setVariableValue}
                  ondefs={setVariables}
                />
              {/await}
            {/if}
          </div>
          </div>
        </div>

        <aside class="right" aria-label="Preview">
          <PreviewPanel
            {fields}
            {previewValues}
            {onAir}
            busy={playBusy}
            {autoOutMs}
            onplayin={playIn}
            onplayout={playOut}
            onpreviewvalue={setPreviewValue}
            onautoout={setAutoOut}
          />
        </aside>
      </div>
    {/if}
  </div>
</div>

{#if Confirm && confirmOpen}
  <Confirm
    title="Delete template?"
    message="“{name}” will be deleted from this project. Backups keep a copy."
    confirmLabel="Delete"
    danger
    busy={deleteBusy}
    onconfirm={confirmDelete}
    oncancel={() => (confirmOpen = false)}
  />
{/if}

<style>
  /* The size container for the stacked layout (a grid can't query its own width). */
  .frame {
    container-type: inline-size;
  }
  .editor {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    /* The content pane's height under the top bar: both columns scroll inside it. */
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
    flex: 0 1 28rem;
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
  /* Quiet save status, right-aligned just before the actions: muted "Saved" / "Saving…", danger
     text only when it could not save. */
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
    grid-template-columns: minmax(0, 55fr) minmax(0, 45fr);
    min-height: 0;
  }
  /* One column pinned to the space it gets: without it the implicit auto column grows to the
     looks row's full width (10 looks ≈ 1360px) and the code runs under the preview. The looks
     row scrolls instead (VariantTabs). */
  .left {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto minmax(0, 1fr);
    min-height: 0;
    min-width: 0;
    border-right: 1px solid var(--line);
  }
  .vpanel {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto minmax(0, 1fr);
    min-height: 0;
    min-width: 0;
  }
  .right {
    min-width: 0;
    min-height: 0;
    overflow: auto;
  }

  .code-hint {
    padding: var(--s-2) var(--s-3);
    border-bottom: 1px solid var(--line);
  }
  .tabs {
    display: flex;
    gap: var(--s-1);
    padding: 0 var(--s-2);
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .tab {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: calc(var(--row-h) + var(--s-1));
    padding: 0 var(--s-3);
    color: var(--text-muted);
    font-weight: 500;
  }
  .tab:hover {
    color: var(--text);
  }
  .tab:focus-visible {
    outline-offset: -2px;
  }
  .tab[aria-selected="true"] {
    color: var(--accent-text);
  }
  .tab::after {
    content: "";
    position: absolute;
    left: var(--s-2);
    right: var(--s-2);
    bottom: -1px;
    height: 2px;
    border-radius: 1px;
    background: var(--accent);
    opacity: 0;
    transform: scaleX(0.6);
    transition:
      opacity var(--dur-fast) var(--ease),
      transform var(--dur-fast) var(--ease);
  }
  .tab[aria-selected="true"]::after {
    opacity: 1;
    transform: scaleX(1);
  }
  .tab-issues {
    color: var(--danger);
    font-size: var(--fs-xs);
  }

  .panel {
    min-height: 0;
    overflow: auto;
  }
  .panel.code {
    overflow: hidden;
    background: var(--surface-1);
  }
  .panel[hidden] {
    display: none;
  }

  @container (max-width: 1000px) {
    .body {
      grid-template-columns: minmax(0, 1fr);
      grid-auto-rows: auto;
    }
    .editor {
      height: auto;
    }
    .left {
      border-right: 0;
      grid-template-rows: auto minmax(28rem, 60vh);
    }
    .right {
      border-top: 1px solid var(--line);
      overflow: visible;
    }
  }
</style>
