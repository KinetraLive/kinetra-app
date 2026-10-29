<!--
  ApiEditor: the Sources page's editor for an API source (kind `api`; its own lazy chunk).
  Simple mode: a CSV source is a "CSV link" (Link = the URL alone; no Format select); a JSON source
  shows its Status and Preview with the "Available in Advanced mode" hint instead of the rest.
    header    name (inline), "CSV link" or "API", mono rows · columns, save status, Duplicate, Delete
    left      Request (RequestSection: URL, method, headers with keychain secrets, POST body),
              Format (JSON / CSV, header row, delimiter), Polling (interval in seconds, min 1, only
              when used, the limit hint), Status (StatusSection: state, last / next fetch, error,
              Retry now, Pause / Resume)
    right     Mapping (MappingSection: Test request, the response tree or CSV columns, the columns)
    bottom    Preview: the mapped rows in the read-only sheet: from the last test (JSON mapped here
              as the mapping changes) or the last good fetch

  Saving: like the data table editor (Autosave, 500 ms, Ctrl/Cmd+S, flushed on unmount; an invalid
  copy isn't saved). Secret values never enter the document: the keychain commands return the
  document with the header's secret id, merged into the working copy.

  While shown (mounted and the window visible) the source counts as in use for polling
  (watchApiSource on; off when hidden or unmounted). The status follows the `api` prop (the page
  patches it from `source-changed`); a new good fetch reloads the cached rows when no test is shown.

  Props: as SourceEditor, plus api: ApiStatus | null.
-->
<script module lang="ts">
  let sheetChunk: Promise<typeof import("$lib/components/sheet/SheetEditor.svelte")> | null = null;
  function loadSheet() {
    sheetChunk ??= import("$lib/components/sheet/SheetEditor.svelte");
    sheetChunk.catch(() => (sheetChunk = null));
    return sheetChunk;
  }
  void loadSheet().catch(() => {});
</script>

<script lang="ts">
  import { onMount, untrack } from "svelte";
  import Copy from "lucide-svelte/icons/copy";
  import RotateCw from "lucide-svelte/icons/rotate-cw";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import { on, toAppError } from "$lib/api";
  import * as apiSources from "$lib/api/api-sources";
  import * as sourcesApi from "$lib/api/sources";
  import AdvancedHint from "$lib/components/AdvancedHint.svelte";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import type { SheetLimits } from "$lib/components/sheet/types";
  import DiskBanner from "$lib/components/templates/editor/DiskBanner.svelte";
  import { Autosave, diskReaction, type SaveState } from "$lib/components/templates/editor/sync";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import {
    SOURCE_CELL_MAX,
    SOURCE_COLUMNS_MAX,
    SOURCE_NAME_MAX,
    SOURCE_ROWS_MAX,
    type ApiFormat,
    type ApiMapping,
    type ApiStatus,
    type ApiTestResult,
    type FilesChanged,
    type SourceDoc,
    type SourceSummary,
  } from "$lib/types";
  import { rowsLabel, summaryOf } from "../model";
  import MappingSection from "./MappingSection.svelte";
  import RequestSection from "./RequestSection.svelte";
  import StatusSection from "./StatusSection.svelte";
  import { ago } from "./status";
  import { now } from "./clock.svelte";
  import { apiBlocked, apiIssues, intervalFrom, mapJson, mergeSecrets, withMapping, type ApiDoc } from "./model";

  let {
    id,
    usedBy,
    api = null,
    onsaved,
    onduplicated,
    ondeleted,
  }: {
    id: string;
    usedBy: readonly string[];
    api?: ApiStatus | null;
    onsaved: (summary: SourceSummary) => void;
    onduplicated: (doc: SourceDoc) => void;
    ondeleted: () => void;
  } = $props();

  const SAVE_MS = 500;
  const LIMITS: SheetLimits = { rows: SOURCE_ROWS_MAX, bytes: null, cell: SOURCE_CELL_MAX, columns: SOURCE_COLUMNS_MAX };
  const uid = $props.id();
  const FORMATS: { value: ApiFormat; label: string }[] = [
    { value: "json", label: "JSON" },
    { value: "csv", label: "CSV" },
  ];
  const DELIMITERS: { value: ApiMapping["csv"]["delimiter"]; label: string }[] = [
    { value: "auto", label: "Detect" },
    { value: ",", label: "Comma ," },
    { value: ";", label: "Semicolon ;" },
    { value: "\t", label: "Tab" },
  ];

  // ---- State --------------------------------------------------------------------------------------

  let loaded = $state(false);
  let loadError = $state<string | null>(null);
  let doc = $state.raw<ApiDoc | null>(null);
  let updatedAt = $state<string | null>(null);

  const issues = $derived(doc ? apiIssues(doc) : null);
  /** Simple mode: a CSV source is a "CSV link" (URL, header row, delimiter, polling, columns); a
   * JSON one shows its status and data, with the Advanced mode hint for the rest. */
  const simple = $derived(!isAdvanced());
  const csvLink = $derived(simple && doc?.format === "csv");
  const jsonHidden = $derived(simple && doc?.format === "json");

  function fail(e: unknown) {
    toasts.push({ kind: "error", message: toAppError(e).message });
  }

  // ---- Save ---------------------------------------------------------------------------------------

  let saveState = $state<SaveState>({ kind: "saved", at: null });
  let disk = $state<"conflict" | "removed" | null>(null);
  let diskBusy = $state(false);
  let deleted = false;

  const docBlocked = $derived(doc && issues ? apiBlocked(doc, issues) : null);
  const saveBlocked = $derived(disk === "conflict" ? "Changed on disk: choose Reload or Keep mine." : docBlocked);

  /** The summary keeps the status the page knows (a save doesn't change it). */
  const summarize = (d: SourceDoc): SourceSummary => ({ ...summaryOf(d, usedBy), api });

  const autosave = new Autosave<SourceDoc>({
    delayMs: SAVE_MS,
    snapshot: () => snapshot(),
    blocked: () => saveBlocked,
    save: async (d) => {
      const saved = await sourcesApi.saveSource(d);
      onsaved(summarize(saved));
      return { updatedAt: saved.updatedAt };
    },
    onState: (s) => {
      if (s.kind === "saved" && s.at) updatedAt = s.at;
      if (s.kind !== saveState.kind || s.kind === "saved" || s.kind === "failed") saveState = s;
    },
    describe: (e) => toAppError(e).message,
  });

  function snapshot(): ApiDoc {
    const d = doc as ApiDoc;
    return { ...d, name: d.name.trim(), updatedAt: updatedAt ?? "" };
  }

  function edit(next: ApiDoc) {
    if (next === doc) return;
    doc = next;
    autosave.touch();
  }

  function saveNow() {
    if (loaded && !deleted) void autosave.saveNow();
  }

  function onWindowKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "s") {
      e.preventDefault();
      saveNow();
    }
  }

  // ---- Secrets ------------------------------------------------------------------------------------

  /** A keychain command's reply: its headers' secret ids into the working copy. */
  function onSecret(reply: SourceDoc) {
    if (!doc || reply.kind !== "api") return;
    doc = { ...doc, request: { ...doc.request, headers: mergeSecrets(doc.request.headers, reply.request.headers) } };
    updatedAt = reply.updatedAt;
  }

  // ---- Test request -------------------------------------------------------------------------------

  let result = $state.raw<ApiTestResult | null>(null);
  let testing = $state(false);
  /** The mapping the server applied for `result` (CSV and large JSON aren't re-mapped here). */
  let testedMapping = $state<string | null>(null);

  async function runTest() {
    if (!doc || testing) return;
    testing = true;
    const d = snapshot();
    try {
      result = await apiSources.testApiSource(d);
    } catch (e) {
      result = { httpStatus: null, headers: [], preview: "", json: null, csvColumns: null, rows: [], hits: {}, error: toAppError(e).message };
    } finally {
      testing = false;
      testedMapping = JSON.stringify(d.mapping);
    }
  }

  const localJson = $derived(doc?.format === "json" && result !== null && result.json !== null && result.json !== undefined);
  const mapped = $derived(localJson && doc && result ? mapJson(result.json, doc.mapping) : null);
  const stale = $derived(!!result && !localJson && doc !== null && testedMapping !== JSON.stringify(doc.mapping));

  /** Preview rows: the test's (mapped here for JSON), else the last good fetch. */
  const preview = $derived.by(() => {
    if (result && !result.error) return mapped ? mapped.rows : result.rows;
    return doc?.rows ?? [];
  });
  const hits = $derived(result && !result.error ? (mapped ? mapped.hits : result.hits) : null);
  const sheetColumns = $derived((doc?.columns ?? []).map(({ key, label }) => ({ key, label })));

  // ---- Polling ------------------------------------------------------------------------------------

  let intervalNote = $state<string | null>(null);
  /** Remounts the interval box after a clamp (it may still show what was typed). */
  let intervalGen = $state(0);

  function setPollSeconds(seconds: number) {
    if (!doc) return;
    const { ms, clamped } = intervalFrom(seconds);
    intervalNote =
      clamped === "min"
        ? "The shortest interval is 1 second; set to 1."
        : clamped === "max"
          ? "The longest interval is 1 hour; set to 3600."
          : null;
    if (clamped) intervalGen++;
    edit({ ...doc, poll: { ...doc.poll, intervalMs: ms } });
  }

  // ---- Status actions -----------------------------------------------------------------------------

  let statusBusy = $state<"retry" | "pause" | null>(null);

  async function retry() {
    statusBusy = "retry";
    try {
      await autosave.flush();
      onsaved(await apiSources.retryApiSource(id));
    } catch (e) {
      fail(e);
    } finally {
      statusBusy = null;
    }
  }

  async function pause(paused: boolean) {
    if (!doc) return;
    statusBusy = "pause";
    try {
      await autosave.flush();
      const s = await apiSources.setApiPaused(id, paused);
      if (doc) doc = { ...doc, poll: { ...doc.poll, paused } };
      onsaved(s);
    } catch (e) {
      fail(e);
    } finally {
      statusBusy = null;
    }
  }

  // A new good fetch: the cached rows (shown when no test is) come from the file.
  let lastOk: number | null = untrack(() => api?.lastOkAt ?? null);
  $effect(() => {
    const at = api?.lastOkAt ?? null;
    if (at === lastOk) return;
    lastOk = at;
    if (!loaded || deleted) return;
    sourcesApi.getSource(id).then(
      (d) => {
        if (doc && d.kind === "api") doc = { ...doc, rows: d.rows, index: d.index, fetchedAt: d.fetchedAt };
      },
      () => {},
    );
  });

  // ---- Duplicate / delete -------------------------------------------------------------------------

  let dupBusy = $state(false);

  async function duplicate() {
    if (dupBusy || !doc) return;
    dupBusy = true;
    try {
      await autosave.flush();
      const copy = await sourcesApi.duplicateSource(id, `${doc.name.trim()} copy`.slice(0, SOURCE_NAME_MAX));
      toasts.push({ kind: "success", message: `Duplicated as “${copy.name}”` });
      onduplicated(copy);
    } catch (e) {
      fail(e);
    } finally {
      dupBusy = false;
    }
  }

  type ConfirmDialogT = typeof import("$lib/components/ConfirmDialog.svelte").default;
  let Confirm = $state.raw<ConfirmDialogT | null>(null);
  let confirmMessage = $state<string | null>(null);
  let deleteBusy = $state(false);

  async function askDelete() {
    if (!doc) return;
    const name = doc.name.trim();
    try {
      Confirm ??= (await import("$lib/components/ConfirmDialog.svelte")).default;
    } catch (e) {
      fail(e);
      return;
    }
    const n = usedBy.length;
    confirmMessage =
      n === 0
        ? `“${name}” will be removed from the project with its stored secrets. No screen uses it. Backups keep a copy of its settings.`
        : `“${name}” will be removed from the project with its stored secrets. ${n === 1 ? "A screen uses" : `${n} screens use`} it; their bound fields fall back to the layer's own values. Backups keep a copy of its settings.`;
  }

  async function confirmDelete() {
    deleteBusy = true;
    try {
      autosave.cancel();
      deleted = true;
      await sourcesApi.deleteSource(id);
      confirmMessage = null;
      toasts.push({ kind: "success", message: `Deleted “${doc?.name ?? ""}”` });
      ondeleted();
    } catch (e) {
      deleted = false;
      fail(e);
    } finally {
      deleteBusy = false;
    }
  }

  // ---- Load, external changes, in-use, teardown -----------------------------------------------------

  function applyDoc(d: SourceDoc) {
    if (d.kind !== "api") throw new Error("Not an API source.");
    doc = d;
    updatedAt = d.updatedAt;
    saveState = { kind: "saved", at: d.updatedAt };
  }

  async function reloadFromDisk(): Promise<boolean> {
    try {
      const d = await sourcesApi.getSource(id);
      autosave.discard();
      applyDoc(d);
    } catch (e) {
      fail(e);
      return false;
    }
    disk = null;
    return true;
  }

  function onFilesChanged(e: FilesChanged) {
    if (!loaded || deleted || !doc) return;
    switch (diskReaction(e, "source", id, autosave.dirty || autosave.saving || disk === "conflict")) {
      case "reload":
        void reloadFromDisk();
        break;
      case "conflict":
        disk = "conflict";
        break;
      case "removed":
        deleted = true;
        autosave.cancel();
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

  /** In use for polling while shown: on when mounted and visible, off when hidden or gone. */
  let watching = false;
  function watch(on: boolean) {
    if (on === watching) return;
    watching = on;
    apiSources.watchApiSource(id, on).catch(() => {});
  }

  onMount(() => {
    let alive = true;
    const off = on("project-files-changed", onFilesChanged);
    const onVisibility = () => watch(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", onVisibility);
    onVisibility();
    sourcesApi.getSource(id).then(
      (d) => {
        if (!alive) return;
        try {
          applyDoc(d);
          loaded = true;
        } catch (e) {
          loadError = toAppError(e).message;
        }
      },
      (e) => {
        if (alive) loadError = toAppError(e).message;
      },
    );
    return () => {
      alive = false;
      off();
      document.removeEventListener("visibilitychange", onVisibility);
      watch(false);
      if (deleted) autosave.cancel();
      else void autosave.flush();
    };
  });

  const savedTitle = $derived(updatedAt ? `Saved ${new Date(updatedAt).toLocaleString()}` : undefined);
  const fetchedMs = $derived(doc?.fetchedAt ? Date.parse(doc.fetchedAt) : NaN);
</script>

<svelte:window onkeydown={onWindowKey} />

{#if loadError}
  <div class="load-error">
    <p class="bad">Couldn't open this source: {loadError}</p>
  </div>
{:else if loaded && doc && issues}
  <div class="editor">
    <header class="bar">
      <span class="name vf vf-within">
        <input
          class="name-input"
          type="text"
          aria-label="Source name"
          autocomplete="off"
          maxlength={SOURCE_NAME_MAX + 20}
          value={doc.name}
          aria-invalid={issues.name ? "true" : undefined}
          aria-describedby={issues.name ? `${uid}-name-err` : undefined}
          oninput={(e) => doc && edit({ ...doc, name: e.currentTarget.value })}
        />
      </span>
      {#if issues.name}<span class="bad small" id="{uid}-name-err">{issues.name}</span>{/if}

      <span class="muted small">{doc.format === "csv" ? "CSV link" : "API"}</span>
      <span class="muted small">
        {rowsLabel(doc.rows.length)} · {doc.columns.length} column{doc.columns.length === 1 ? "" : "s"}
      </span>

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
    </header>

    {#if disk}
      <DiskBanner what="source" state={disk} busy={diskBusy} onreload={diskReload} onkeep={diskKeep} onclose={ondeleted} />
    {/if}

    <div class="body">
      <div class="cols">
        <div class="col">
          {#if jsonHidden}
          <StatusSection status={api} paused={doc.poll.paused} busy={statusBusy} onretry={retry} onpause={pause} />
          <AdvancedHint feature="The request and mapping" />
          {:else}
          <RequestSection
            sourceId={id}
            {doc}
            {issues}
            onedit={edit}
            flush={() => autosave.flush()}
            onsecret={onSecret}
            linkOnly={csvLink}
          />

          <section class="section" aria-labelledby="{uid}-format">
            <h3 class="section-label" id="{uid}-format">Format</h3>
            <div class="line">
              {#if !csvLink}<Select aria-label="Format" value={doc.format} options={FORMATS} onchange={(v) => doc && edit({ ...doc, format: v })} />{/if}
              {#if doc.format === "csv"}
                <Switch
                  label="First row is the header"
                  checked={doc.mapping.csv.header}
                  onchange={(v) => doc && edit({ ...doc, mapping: { ...doc.mapping, csv: { ...doc.mapping.csv, header: v } } })}
                />
                <Select
                  aria-label="Delimiter"
                  value={doc.mapping.csv.delimiter}
                  options={DELIMITERS}
                  onchange={(v) => doc && edit({ ...doc, mapping: { ...doc.mapping, csv: { ...doc.mapping.csv, delimiter: v } } })}
                />
              {/if}
            </div>
          </section>

          <section class="section" aria-labelledby="{uid}-poll">
            <h3 class="section-label" id="{uid}-poll">Polling</h3>
            <div class="line">
              <label class="small muted" for="{uid}-interval">Every</label>
              {#key intervalGen}<NumberInput
                id="{uid}-interval"
                value={doc.poll.intervalMs / 1000}
                min={0}
                max={3600}
                step={0.1}
                digits={6}
                suffix="s"
                aria-describedby="{uid}-poll-hint"
                onchange={setPollSeconds}
              />{/key}
              <Switch
                label="Poll only when used"
                checked={doc.poll.onlyWhenUsed}
                onchange={(v) => doc && edit({ ...doc, poll: { ...doc.poll, onlyWhenUsed: v } })}
              />
            </div>
            {#if intervalNote}<p class="warn small" role="status">{intervalNote}</p>{/if}
            <p class="small muted" id="{uid}-poll-hint">Once a second at most. For live data, use a source that pushes updates.</p>
          </section>

          <StatusSection status={api} paused={doc.poll.paused} busy={statusBusy} onretry={retry} onpause={pause} />
          {/if}
        </div>

        {#if !jsonHidden}
        <div class="col">
          <MappingSection
            mapping={doc.mapping}
            format={doc.format}
            url={doc.request.url}
            {issues}
            {result}
            {testing}
            {hits}
            rowCount={preview.length}
            link={csvLink}
            ontest={runTest}
            onmapping={(m) => doc && edit(withMapping(doc, m))}
          />
        </div>
        {/if}
      </div>

      <section class="section preview" aria-labelledby="{uid}-preview">
        <div class="line">
          <h3 class="section-label" id="{uid}-preview">Preview</h3>
          <span class="small muted tnum">
            {#if result && !result.error}
              From the test request
            {:else if !Number.isNaN(fetchedMs)}
              Last fetch, {ago(fetchedMs, now())}
            {/if}
          </span>
          {#if stale}<span class="small warn">The mapping changed: test the request again to see it.</span>{/if}
          {#if mapped?.error}<span class="small warn">{mapped.error}</span>{/if}
        </div>
        {#if preview.length === 0 || sheetColumns.length === 0}
          <p class="small muted">
            {sheetColumns.length === 0 ? "Map at least one column to see rows here." : "No rows yet: test the request, or wait for the first fetch."}
          </p>
        {:else}
          {#await loadSheet()}
            <!-- Loaded with this chunk. -->
          {:then { default: Sheet }}
            <div class="sheet" style:--n={preview.length}>
              <Sheet
                title="Preview of {doc.name}"
                hideTitle
                readonly
                columns={sheetColumns}
                rows={preview}
                limits={LIMITS}
                onchange={() => {}}
              />
            </div>
          {:catch e}
            <p class="bad small">Couldn't open the table: {toAppError(e).message}</p>
          {/await}
        {/if}
      </section>
    </div>
  </div>
{/if}

{#if Confirm && confirmMessage && doc}
  <Confirm
    title="Delete source?"
    message={confirmMessage}
    confirmLabel="Delete"
    danger
    busy={deleteBusy}
    onconfirm={confirmDelete}
    oncancel={() => (confirmMessage = null)}
  />
{/if}

<style>
  .editor {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
  }
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
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .cols {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1.3fr);
    border-bottom: 1px solid var(--line);
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: var(--s-5);
    min-width: 0;
    padding: var(--s-4) var(--s-5) var(--s-5);
  }
  .col + .col {
    border-left: 1px solid var(--line);
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
  }
  .preview {
    padding: var(--s-4) var(--s-5) var(--s-5);
  }
  .sheet {
    display: flex;
    flex-direction: column;
    height: min(22rem, calc(var(--n) * var(--row-h) + 5.5rem));
    min-height: 0;
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line);
    overflow: hidden;
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
  .warn {
    color: var(--warning);
  }
  .load-error {
    padding: var(--s-6);
  }
  @media (max-width: 1100px) {
    .cols {
      grid-template-columns: minmax(0, 1fr);
    }
    .col + .col {
      border-left: 0;
      border-top: 1px solid var(--line);
    }
  }
</style>
