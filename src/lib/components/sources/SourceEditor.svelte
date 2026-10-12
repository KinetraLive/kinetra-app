<!--
  SourceEditor: the Sources page's right column, the editor for one data table (kind `text`).
    header    name (inline), "Data table", mono counts, save status, Duplicate, Delete
    sheet     the spreadsheet editor (SheetEditor, a lazy chunk that starts loading with this one)
              with editable columns (label, key, Text/Image, add, remove, reorder), and "Make
              current" for the active row in its toolbar. The current row is marked.

  Props:
    id: string                           the source id (remount the component to switch sources)
    usedBy: string[]                     screen ids bound to it (from the list)
    onsaved: (summary) => void           after a save or a step: the list entry as it is now
    onduplicated: (doc) => void          a copy was made
    ondeleted: () => void                deleted here, or Close on "Deleted on disk"

  Saving: every sheet change replaces the working copy and touches the shared Autosave (500 ms
  after the last edit, one save in flight; Ctrl/Cmd+S saves at once; unmount flushes). An invalid
  copy is never saved; the status says why. The current row (`index`) follows its row through
  moves and deletes (the sheet's `origin`).

  Column keys: while screens use the source, keys loaded from disk are locked in the sheet so their
  bindings keep working; new columns' keys follow their label until edited.

  Changed on disk: no unsaved edits → reload silently; unsaved edits → DiskBanner (Reload / Keep
  mine); deleted → banner with Close; invalid → toast, the working copy stays.
-->
<script module lang="ts">
  let sheetChunk: Promise<typeof import("$lib/components/sheet/SheetEditor.svelte")> | null = null;
  function loadSheet() {
    sheetChunk ??= import("$lib/components/sheet/SheetEditor.svelte");
    sheetChunk.catch(() => (sheetChunk = null));
    return sheetChunk;
  }
  // Starts with this chunk, in parallel with the source's load.
  void loadSheet().catch(() => {});
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import Copy from "lucide-svelte/icons/copy";
  import RotateCw from "lucide-svelte/icons/rotate-cw";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import { on, toAppError } from "$lib/api";
  import * as sourcesApi from "$lib/api/sources";
  import Button from "$lib/components/Button.svelte";
  import type { SheetLimits, SheetRow } from "$lib/components/sheet/types";
  import DiskBanner from "$lib/components/templates/editor/DiskBanner.svelte";
  import { Autosave, diskReaction, type SaveState } from "$lib/components/templates/editor/sync";
  import { toasts } from "$lib/stores/toasts.svelte";
  import {
    SOURCE_CELL_MAX,
    SOURCE_COLUMNS_MAX,
    SOURCE_NAME_MAX,
    SOURCE_ROWS_MAX,
    type FilesChanged,
    type SourceColumn,
    type SourceDoc,
    type SourceSummary,
  } from "$lib/types";
  import { blockedReason, rowsLabel, sourceIssues, summaryOf } from "./model";

  let {
    id,
    usedBy,
    onsaved,
    onduplicated,
    ondeleted,
  }: {
    id: string;
    usedBy: readonly string[];
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
  /** The working copy; replaced wholesale on every edit. */
  let doc = $state.raw<SourceDoc | null>(null);
  let updatedAt = $state<string | null>(null);
  /** Column keys as loaded; fixed while screens use the source. */
  let loadedKeys = $state.raw<string[]>([]);

  const rows = $derived(doc?.rows ?? []);
  const columns = $derived(doc?.columns ?? []);
  const current = $derived(doc?.index ?? 0);
  const issues = $derived(doc ? sourceIssues(doc) : null);
  const lockedKeys = $derived(usedBy.length > 0 ? loadedKeys : []);

  function fail(e: unknown) {
    toasts.push({ kind: "error", message: toAppError(e).message });
  }

  // ---- Save ---------------------------------------------------------------------------------------

  let saveState = $state<SaveState>({ kind: "saved", at: null });
  let disk = $state<"conflict" | "removed" | null>(null);
  let diskBusy = $state(false);
  let deleted = false;

  const docBlocked = $derived(doc && issues ? blockedReason(doc, issues) : null);
  const saveBlocked = $derived(
    disk === "conflict" ? "Changed on disk: choose Reload or Keep mine." : docBlocked,
  );

  const autosave = new Autosave<SourceDoc>({
    delayMs: SAVE_MS,
    snapshot: () => ({ ...(doc as SourceDoc), name: (doc as SourceDoc).name.trim(), updatedAt: updatedAt ?? "" }),
    blocked: () => saveBlocked,
    save: async (d) => {
      const saved = await sourcesApi.saveSource(d);
      onsaved(summaryOf(saved, usedBy));
      return { updatedAt: saved.updatedAt };
    },
    onState: (s) => {
      if (s.kind === "saved" && s.at) updatedAt = s.at;
      if (s.kind !== saveState.kind || s.kind === "saved" || s.kind === "failed") saveState = s;
    },
    describe: (e) => toAppError(e).message,
  });

  function edit(next: SourceDoc) {
    if (next === doc) return;
    doc = next;
    autosave.touch();
  }

  /** A sheet change: the current row follows its row (`origin`); a deleted one keeps its place. */
  function onSheet(nextRows: SheetRow[], nextColumns: SourceColumn[], origin: readonly number[]) {
    if (!doc) return;
    const old = doc.index;
    let index = origin.indexOf(old);
    if (index < 0) index = origin.filter((o) => o >= 0 && o < old).length;
    index = nextRows.length === 0 ? 0 : Math.min(index, nextRows.length - 1);
    edit({ ...doc, rows: nextRows, columns: nextColumns, index });
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

  async function makeCurrent(i: number) {
    if (!doc || i === doc.index) return;
    const prev = doc.index;
    doc = { ...doc, index: i }; // optimistic; later saves carry it
    try {
      await autosave.flush(); // the step applies to what is saved
      const s = await sourcesApi.stepSource(id, { to: "index", index: i });
      if (doc) doc = { ...doc, index: s.index };
      onsaved(s);
    } catch (e) {
      if (doc && doc.index === i) doc = { ...doc, index: prev };
      fail(e);
    }
  }

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
    let screens = [...usedBy];
    if (screens.length > 0) {
      try {
        const { listScreenDocs } = await import("$lib/api/screens");
        const names = new Map((await listScreenDocs()).map((s) => [s.id, s.name]));
        screens = screens.map((s) => names.get(s) ?? s);
      } catch {
        // Fall back to the ids.
      }
    }
    confirmMessage =
      screens.length === 0
        ? `“${name}” will be removed from the project. No screen uses it. Backups keep a copy.`
        : `“${name}” will be removed from the project. It is used by ${screens.length === 1 ? "the screen" : `${screens.length} screens:`} ${screens.map((s) => `“${s}”`).join(", ")}; their bound fields fall back to the layer's own values. Backups keep a copy.`;
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

  // ---- Load, external changes, teardown -------------------------------------------------------------

  function applyDoc(d: SourceDoc) {
    doc = d;
    updatedAt = d.updatedAt;
    loadedKeys = d.columns.map((c) => c.key);
    saveState = { kind: "saved", at: d.updatedAt };
  }

  async function reloadFromDisk(): Promise<boolean> {
    let d: SourceDoc;
    try {
      d = await sourcesApi.getSource(id);
    } catch (e) {
      fail(e);
      return false;
    }
    autosave.discard();
    applyDoc(d);
    disk = null;
    return true;
  }

  function onFilesChanged(e: FilesChanged) {
    if (!loaded || deleted || !doc) return;
    switch (diskReaction(e, "source", id, autosave.dirty || autosave.saving || disk === "conflict")) {
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
    const off = on("project-files-changed", onFilesChanged);
    sourcesApi.getSource(id).then(
      (d) => {
        if (!alive) return;
        applyDoc(d);
        loaded = true;
      },
      (e) => {
        if (alive) loadError = toAppError(e).message;
      },
    );
    return () => {
      alive = false;
      off();
      if (deleted) autosave.cancel();
      else void autosave.flush();
    };
  });

  const savedTitle = $derived(updatedAt ? `Saved ${new Date(updatedAt).toLocaleString()}` : undefined);
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

      <span class="muted small">Data table</span>
      <span class="muted small">
        {rowsLabel(rows.length)} · {columns.length} column{columns.length === 1 ? "" : "s"}
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
      <DiskBanner
        what="source"
        state={disk}
        busy={diskBusy}
        onreload={diskReload}
        onkeep={diskKeep}
        onclose={ondeleted}
      />
    {/if}

    {#await loadSheet()}
      <!-- Loaded with this chunk: nothing shows in between. -->
    {:then { default: Sheet }}
      <div class="sheet">
        <Sheet
          title="Rows of {doc.name}"
          hideTitle
          {columns}
          {rows}
          limits={LIMITS}
          editableColumns
          currentRow={rows.length > 0 ? current : null}
          {lockedKeys}
          onchange={onSheet}
        >
          {#snippet toolbar(active: number | null)}
            {#if active !== null}
              <Button size="sm" variant="ghost" disabled={active === current} onclick={() => void makeCurrent(active)}>
                Make row {active + 1} current
              </Button>
            {/if}
            {#if rows.length > 0}
              <span class="small muted">Current row <span class="text">{current + 1}</span></span>
            {/if}
          {/snippet}
        </Sheet>
      </div>
    {:catch e}
      <p class="load-error bad">Couldn't open the table: {toAppError(e).message}</p>
    {/await}
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
  .sheet {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  .small {
    font-size: var(--fs-sm);
  }
  .muted {
    color: var(--text-muted);
  }
  .text {
    color: var(--text);
  }
  .bad {
    color: var(--danger);
  }
  .load-error {
    padding: var(--s-6);
  }
</style>
