<!--
  SocketEditor: the Sources page's editor for a live data source (kind `socket`, "Live data
  (WebSocket)"; its own lazy chunk; Advanced mode, Simple shows the status and data with the
  Advanced mode hint).
    header    name (inline), "Live data", rows · columns, save status, Duplicate, Delete
    left      Connection (URL, Send on connect), Filter (only messages where <pointer> equals <value>),
              Updates (a second, 1–30), Status (state, last message, next attempt, dropped messages,
              the problem, Connect now, Pause / Resume)
    right     Mapping (MappingSection in socket mode: Test connection, the message tree, the columns)
    bottom    Preview: the mapped rows in the read-only sheet: from the test (mapped here as the
              mapping changes) or the live rows

  Saving: like the API editor (Autosave, 500 ms, Ctrl/Cmd+S, flushed on unmount; an invalid copy
  isn't saved). The rows are the connection's: a save keeps the live ones.

  While shown (mounted and the window visible) the source counts as in use, so it connects
  (watchSocketSource on; off when hidden or unmounted). The status follows the `socket` prop (the
  page patches it from `source-changed`, at most 4 a second); new rows are read then, unless a test
  is shown.

  Props: as SourceEditor, plus socket: SocketStatus | null.
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
  import { onMount } from "svelte";
  import Copy from "lucide-svelte/icons/copy";
  import Pause from "lucide-svelte/icons/pause";
  import Play from "lucide-svelte/icons/play";
  import RotateCw from "lucide-svelte/icons/rotate-cw";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import { on, toAppError } from "$lib/api";
  import * as socketApi from "$lib/api/socket-sources";
  import * as sourcesApi from "$lib/api/sources";
  import AdvancedHint from "$lib/components/AdvancedHint.svelte";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import type { SheetLimits } from "$lib/components/sheet/types";
  import DiskBanner from "$lib/components/templates/editor/DiskBanner.svelte";
  import { Autosave, diskReaction, type SaveState } from "$lib/components/templates/editor/sync";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import {
    SOCKET_RATE_MAX,
    SOCKET_RATE_MIN,
    SOURCE_CELL_MAX,
    SOURCE_COLUMNS_MAX,
    SOURCE_NAME_MAX,
    SOURCE_ROWS_MAX,
    type FilesChanged,
    type SocketStatus,
    type SocketTestResult,
    type SourceDoc,
    type SourceSummary,
  } from "$lib/types";
  import MappingSection from "../api/MappingSection.svelte";
  import { hold, now } from "../api/clock.svelte";
  import { mapJson, withMapping } from "../api/model";
  import { ago, until } from "../api/status";
  import { rowsLabel, summaryOf } from "../model";
  import { rateFrom, socketBlocked, socketIssues, withFilter, type SocketDoc } from "./model";
  import { socketLong } from "./status";

  let {
    id,
    usedBy,
    socket = null,
    onsaved,
    onduplicated,
    ondeleted,
  }: {
    id: string;
    usedBy: readonly string[];
    socket?: SocketStatus | null;
    onsaved: (summary: SourceSummary) => void;
    onduplicated: (doc: SourceDoc) => void;
    ondeleted: () => void;
  } = $props();

  const SAVE_MS = 500;
  const LIMITS: SheetLimits = { rows: SOURCE_ROWS_MAX, bytes: null, cell: SOURCE_CELL_MAX, columns: SOURCE_COLUMNS_MAX };
  const uid = $props.id();

  // ---- State --------------------------------------------------------------------------------------

  let loaded = $state(false);
  let loadError = $state<string | null>(null);
  let doc = $state.raw<SocketDoc | null>(null);
  let updatedAt = $state<string | null>(null);

  const issues = $derived(doc ? socketIssues(doc) : null);
  const simple = $derived(!isAdvanced());

  function fail(e: unknown) {
    toasts.push({ kind: "error", message: toAppError(e).message });
  }

  // ---- Save ---------------------------------------------------------------------------------------

  let saveState = $state<SaveState>({ kind: "saved", at: null });
  let disk = $state<"conflict" | "removed" | null>(null);
  let diskBusy = $state(false);
  let deleted = false;

  const docBlocked = $derived(doc && issues ? socketBlocked(doc, issues) : null);
  const saveBlocked = $derived(disk === "conflict" ? "Changed on disk: choose Reload or Keep mine." : docBlocked);

  /** The summary keeps the status the page knows (a save doesn't change it). */
  const summarize = (d: SourceDoc): SourceSummary => ({ ...summaryOf(d, usedBy), socket });

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

  function snapshot(): SocketDoc {
    const d = doc as SocketDoc;
    return { ...d, name: d.name.trim(), updatedAt: updatedAt ?? "" };
  }

  function edit(next: SocketDoc) {
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

  // ---- Test connection ----------------------------------------------------------------------------

  let result = $state.raw<SocketTestResult | null>(null);
  let testing = $state(false);

  async function runTest() {
    if (!doc || testing) return;
    testing = true;
    try {
      result = await socketApi.testSocketSource(snapshot());
    } catch (e) {
      result = { messages: [], json: null, rows: [], hits: {}, seen: 0, matched: 0, error: toAppError(e).message };
    } finally {
      testing = false;
    }
  }

  const mapped = $derived(doc && result?.json != null ? mapJson(result.json, doc.mapping) : null);

  /** Preview rows: the test's (mapped here), else the live ones. */
  const preview = $derived.by(() => {
    if (mapped && !mapped.error) return mapped.rows;
    return doc?.rows ?? [];
  });
  const hits = $derived(mapped && !mapped.error ? mapped.hits : null);
  const sheetColumns = $derived((doc?.columns ?? []).map(({ key, label }) => ({ key, label })));
  const testSummary = $derived(
    result && !testing
      ? result.seen === 0
        ? null
        : `${result.seen} ${result.seen === 1 ? "message" : "messages"}${doc?.filter ? `, ${result.matched} matched` : ""}`
      : null,
  );

  // ---- Settings -----------------------------------------------------------------------------------

  let rateNote = $state<string | null>(null);
  let rateGen = $state(0);

  function setRate(n: number) {
    if (!doc) return;
    const { rate, clamped } = rateFrom(n);
    rateNote = clamped ? `Updates are ${SOCKET_RATE_MIN} to ${SOCKET_RATE_MAX} a second; set to ${rate}.` : null;
    if (clamped) rateGen++;
    edit({ ...doc, rate });
  }

  function setSend(text: string) {
    if (!doc) return;
    if (text === "") {
      const next: SocketDoc = { ...doc };
      delete next.sendOnConnect;
      edit(next);
    } else edit({ ...doc, sendOnConnect: text });
  }

  // ---- Status -------------------------------------------------------------------------------------

  // Relative times re-read once a second while shown.
  $effect(() => hold());
  const label = $derived(socketLong(socket));
  const isPaused = $derived(!!doc?.paused || socket?.state === "paused");
  const stamp = (ms: number) => new Date(ms).toLocaleString();

  let statusBusy = $state<"retry" | "pause" | null>(null);

  async function retry() {
    statusBusy = "retry";
    try {
      await autosave.flush();
      onsaved(await socketApi.retrySocketSource(id));
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
      const s = await socketApi.setSocketPaused(id, paused);
      if (doc) doc = { ...doc, paused };
      onsaved(s);
    } catch (e) {
      fail(e);
    } finally {
      statusBusy = null;
    }
  }

  // New rows: read them when the status says so (at most 4 a second), unless a test is shown.
  let lastRows: number | null = null;
  let lastAt: number | null = null;
  $effect(() => {
    const rows = socket?.rows ?? null;
    const at = socket?.lastMessageAt ?? null;
    if (rows === lastRows && at === lastAt) return;
    lastRows = rows;
    lastAt = at;
    if (!loaded || deleted || result) return;
    sourcesApi.getSource(id).then(
      (d) => {
        if (doc && d.kind === "socket") doc = { ...doc, rows: d.rows, index: d.index };
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
        ? `“${name}” will be removed from the project. No screen uses it. Backups keep a copy of its settings.`
        : `“${name}” will be removed from the project. ${n === 1 ? "A screen uses" : `${n} screens use`} it; their bound fields fall back to the layer's own values. Backups keep a copy of its settings.`;
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
    if (d.kind !== "socket") throw new Error("Not a live data source.");
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

  /** In use while shown: on when mounted and visible, off when hidden or gone. */
  let watching = false;
  function watch(on: boolean) {
    if (on === watching) return;
    watching = on;
    socketApi.watchSocketSource(id, on).catch(() => {});
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
</script>

<svelte:window onkeydown={onWindowKey} />

{#snippet status()}
  <section class="section" aria-labelledby="{uid}-status">
    <h3 class="section-label" id="{uid}-status">Status</h3>
    <dl class="facts">
      <dt>State</dt>
      <dd class="state" data-socket-state={socket?.state ?? "none"}>
        <TallyDot state={label.tally} />
        <span>{label.text}</span>
      </dd>
      <dt>Last message</dt>
      <dd>
        {#if socket?.lastMessageAt}
          <span class="tnum" title={stamp(socket.lastMessageAt)}>{ago(socket.lastMessageAt, now())}</span>
        {:else}
          <span class="muted">None yet</span>
        {/if}
      </dd>
      {#if socket?.state === "waiting" && socket.nextAt}
        <dt>Next try</dt>
        <dd><span class="tnum" title={stamp(socket.nextAt)}>{until(socket.nextAt, now())}</span></dd>
      {/if}
      <dt>Rows</dt>
      <dd class="tnum">{socket?.rows ?? doc?.rows.length ?? 0}</dd>
      {#if socket && socket.dropped > 0}
        <dt>Dropped</dt>
        <dd><span class="warn tnum">{socket.dropped} {socket.dropped === 1 ? "message" : "messages"}</span> <span class="muted">(over 200 a second)</span></dd>
      {/if}
    </dl>
    {#if socket?.message}
      <p class="small" class:bad={socket.state === "error"} class:muted={socket.state !== "error"} role="status">{socket.message}</p>
    {/if}
    <div class="line">
      <Button size="sm" variant="secondary" loading={statusBusy === "retry"} disabled={isPaused || !doc?.url.trim()} onclick={retry}>
        {#snippet icon()}<RotateCw size={14} />{/snippet}
        Connect now
      </Button>
      {#if isPaused}
        <Button size="sm" variant="ghost" loading={statusBusy === "pause"} onclick={() => pause(false)}>
          {#snippet icon()}<Play size={14} />{/snippet}
          Resume
        </Button>
      {:else}
        <Button size="sm" variant="ghost" loading={statusBusy === "pause"} onclick={() => pause(true)}>
          {#snippet icon()}<Pause size={14} />{/snippet}
          Pause
        </Button>
      {/if}
    </div>
  </section>
{/snippet}

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

      <span class="muted small">Live data</span>
      <span class="muted small">
        {rowsLabel(doc.rows.length)} · {doc.columns.length} column{doc.columns.length === 1 ? "" : "s"}
      </span>

      <span class="save small" role="status">
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
          {#if simple}
            {@render status()}
            <AdvancedHint feature="The connection and mapping" />
          {:else}
            <section class="section" aria-labelledby="{uid}-conn">
              <h3 class="section-label" id="{uid}-conn">Connection</h3>
              <label class="small muted" for="{uid}-url">URL</label>
              <span class="input vf vf-within">
                <input
                  id="{uid}-url"
                  class="mono"
                  type="url"
                  autocomplete="off"
                  spellcheck="false"
                  placeholder="ws://localhost:8080"
                  value={doc.url}
                  aria-invalid={issues.url ? "true" : undefined}
                  aria-describedby="{uid}-url-hint"
                  oninput={(e) => doc && edit({ ...doc, url: e.currentTarget.value })}
                />
              </span>
              {#if issues.url}
                <p class="bad small" id="{uid}-url-hint">{issues.url}</p>
              {:else}
                <p class="small muted" id="{uid}-url-hint">This computer or your local network.</p>
              {/if}
              <label class="small muted" for="{uid}-send">Send on connect</label>
              <span class="input area vf vf-within">
                <textarea
                  id="{uid}-send"
                  class="mono"
                  rows="2"
                  spellcheck="false"
                  value={doc.sendOnConnect ?? ""}
                  aria-invalid={issues.send ? "true" : undefined}
                  oninput={(e) => setSend(e.currentTarget.value)}
                ></textarea>
              </span>
              {#if issues.send}<p class="bad small">{issues.send}</p>{/if}
            </section>

            <section class="section" aria-labelledby="{uid}-filter">
              <h3 class="section-label" id="{uid}-filter">Filter</h3>
              <div class="line">
                <label class="small muted" for="{uid}-fptr">Only messages where</label>
                <span class="input ptr vf vf-within">
                  <input
                    id="{uid}-fptr"
                    class="mono"
                    type="text"
                    autocomplete="off"
                    spellcheck="false"
                    placeholder="/type"
                    value={doc.filter?.pointer ?? ""}
                    aria-invalid={issues.filter ? "true" : undefined}
                    oninput={(e) => doc && edit(withFilter(doc, e.currentTarget.value, doc.filter?.equals ?? ""))}
                  />
                </span>
                <label class="small muted" for="{uid}-feq">equals</label>
                <span class="input grow vf vf-within">
                  <input
                    id="{uid}-feq"
                    class="mono"
                    type="text"
                    autocomplete="off"
                    spellcheck="false"
                    value={doc.filter?.equals ?? ""}
                    oninput={(e) => doc && edit(withFilter(doc, doc.filter?.pointer ?? "", e.currentTarget.value))}
                  />
                </span>
              </div>
              {#if issues.filter}<p class="bad small">{issues.filter}</p>{/if}
            </section>

            <section class="section" aria-labelledby="{uid}-rate">
              <h3 class="section-label" id="{uid}-rate">Updates</h3>
              <div class="line">
                {#key rateGen}<NumberInput
                  id="{uid}-rate-input"
                  aria-labelledby="{uid}-rate"
                  value={doc.rate}
                  min={SOCKET_RATE_MIN}
                  max={SOCKET_RATE_MAX}
                  step={1}
                  digits={2}
                  onchange={setRate}
                />{/key}
                <span class="small muted">a second, at most</span>
              </div>
              {#if rateNote}<p class="warn small" role="status">{rateNote}</p>{/if}
            </section>

            {@render status()}
          {/if}
        </div>

        {#if !simple}
          <div class="col">
            <MappingSection
              mapping={doc.mapping}
              format="json"
              url={doc.url}
              {issues}
              {result}
              {testing}
              {hits}
              rowCount={preview.length}
              socket
              ontest={runTest}
              onmapping={(m) => doc && edit(withMapping(doc, m))}
            />
            {#if testSummary}<p class="small muted tnum" role="status">{testSummary}</p>{/if}
          </div>
        {/if}
      </div>

      <section class="section preview" aria-labelledby="{uid}-preview">
        <div class="line">
          <h3 class="section-label" id="{uid}-preview">Preview</h3>
          <span class="small muted">
            {#if mapped && !mapped.error}From the test connection{:else if doc.rows.length > 0}Live{/if}
          </span>
          {#if mapped?.error}<span class="small warn">{mapped.error}</span>{/if}
        </div>
        {#if preview.length === 0 || sheetColumns.length === 0}
          <p class="small muted">
            {sheetColumns.length === 0 ? "Map at least one column to see rows here." : "No rows yet: test the connection, or wait for data."}
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
  .save {
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
  .section p {
    margin: 0;
  }
  .line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
  }
  .grow {
    flex: 1;
    min-width: 6rem;
  }
  .input {
    --vf-radius: var(--r-control);
    display: flex;
    height: var(--control-h);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .input.area {
    height: auto;
  }
  .input.ptr {
    flex: 0 1 10rem;
    min-width: 6rem;
  }
  .input input,
  .input textarea {
    flex: 1;
    width: 100%;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
    font-size: var(--fs-sm);
  }
  .input textarea {
    padding: var(--s-2);
    resize: vertical;
  }
  .input:has([aria-invalid="true"]) {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .facts {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: var(--s-1) var(--s-4);
    margin: 0;
    font-size: var(--fs-sm);
  }
  .facts dt {
    color: var(--text-muted);
  }
  .facts dd {
    margin: 0;
    min-width: 0;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
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
