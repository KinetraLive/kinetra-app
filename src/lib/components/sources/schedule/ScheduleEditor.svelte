<!--
  ScheduleEditor: the Sources page's right column for a schedule source (a run of show). Lazy chunk.
    header    name (inline), kind, mono item count, save status, Duplicate, Delete
    toolbar   Add item, Paste rows, Import CSV…, Shift all by (minutes, negative = earlier), Ripple,
              Status labels (opens the six label inputs, ≤ 32 characters each)
    table     items sorted by start and grouped by day: status badge, title, subtitle, location,
              start (date + time), end (time; the next day when not after the start), length in
              minutes, delay (read-only: set from Control) and Duplicate / Delete per item

  Status badges come from the server (scheduleView, then `schedule-changed`): a TallyDot with the
  label text (NOW program, UP NEXT preview, DELAYED warning, PREVIOUS muted, CANCELLED muted and
  struck through, UPCOMING muted). Colour never stands alone.

  Paste: tab- or comma-separated rows (title, start, end, location, or a header naming them) pasted
  anywhere in the table, "Paste rows" (the clipboard) or "Import CSV…" (a file) add items (see
  ./schedule.ts). Times are HH:MM (on the day of the row before) or a full date and time.

  Props:
    id: string                        the source id (remount the component to switch sources)
    usedBy: string[]                  screen ids bound to it (from the list)
    onsaved: (summary) => void        after a save: the list entry as it is now
    onduplicated: (doc) => void       a copy was made
    ondeleted: () => void             deleted here, or Close on "Deleted on disk"

  Saving: Autosave (500 ms after the last edit, one save in flight, Ctrl/Cmd+S at once, unmount
  flushes). An invalid copy is never saved; the status says why and the bad input is marked.
  Changed on disk: like the text source editor (reload silently, or DiskBanner with unsaved edits).
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import ClipboardPaste from "lucide-svelte/icons/clipboard-paste";
  import Copy from "lucide-svelte/icons/copy";
  import FileUp from "lucide-svelte/icons/file-up";
  import Plus from "lucide-svelte/icons/plus";
  import RotateCw from "lucide-svelte/icons/rotate-cw";
  import Tags from "lucide-svelte/icons/tags";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import { on, toAppError } from "$lib/api";
  import { scheduleView } from "$lib/api/recipes";
  import * as sourcesApi from "$lib/api/sources";
  import Button from "$lib/components/Button.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import TimeInput from "$lib/components/inputs/TimeInput.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import DiskBanner from "$lib/components/templates/editor/DiskBanner.svelte";
  import { Autosave, diskReaction, type SaveState } from "$lib/components/templates/editor/sync";
  import { toasts } from "$lib/stores/toasts.svelte";
  import {
    SCHEDULE_ITEMS_MAX,
    SOURCE_NAME_MAX,
    type FilesChanged,
    type ScheduleDoc,
    type ScheduleItemView,
    type ScheduleLabels,
    type ScheduleView,
    type SourceSummary,
  } from "$lib/types";
  import { isMultiCell, parseDelimited } from "../paste";
  import {
    DURATION_MAX,
    LABEL_KEYS,
    LABEL_MAX,
    STATUS_LOOK,
    addItem,
    applySchedulePaste,
    canAddItem,
    datePart,
    dayLabel,
    deleteItem,
    duplicateItem,
    duration,
    groupByDay,
    itemsLabel,
    localDateTime,
    scheduleBlocked,
    scheduleIssues,
    scheduleSummary,
    setDuration,
    setEndTime,
    setStart,
    setText,
    shiftAll,
    timePart,
    validDateTime,
  } from "./schedule";

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
    onduplicated: (doc: { id: string }) => void;
    ondeleted: () => void;
  } = $props();

  const SAVE_MS = 500;
  const SHIFT_MAX = 7 * 24 * 60;
  const uid = $props.id();

  // ---- State --------------------------------------------------------------------------------------

  let loaded = $state(false);
  let loadError = $state<string | null>(null);
  /** The working copy; replaced wholesale on every edit. Items are kept sorted by start. */
  let doc = $state.raw<ScheduleDoc | null>(null);
  let updatedAt = $state<string | null>(null);
  let view = $state.raw<ScheduleView | null>(null);
  let shiftBy = $state(15);
  let labelsOpen = $state(false);
  let table: HTMLDivElement | undefined = $state();
  let fileInput: HTMLInputElement | undefined = $state();

  const items = $derived(doc?.items ?? []);
  const groups = $derived(groupByDay(items));
  const issues = $derived(doc ? scheduleIssues(doc) : null);
  const statusOf = $derived(new Map<string, ScheduleItemView>((view?.items ?? []).map((v) => [v.id, v])));
  const labelIssueCount = $derived(issues ? Object.keys(issues.labels).length : 0);

  function fail(e: unknown) {
    toasts.push({ kind: "error", message: toAppError(e).message });
  }

  // ---- Save ---------------------------------------------------------------------------------------

  let saveState = $state<SaveState>({ kind: "saved", at: null });
  let disk = $state<"conflict" | "removed" | null>(null);
  let diskBusy = $state(false);
  let deleted = false;

  const docBlocked = $derived(doc && issues ? scheduleBlocked(doc, issues) : null);
  /** "<itemId>/<time|end>" → why the typed time isn't one (TimeInput keeps it in the box). */
  let timeErrors = $state.raw<ReadonlyMap<string, string>>(new Map());
  function setTimeError(key: string, reason: string | null) {
    if ((timeErrors.get(key) ?? null) === reason) return;
    const next = new Map(timeErrors);
    if (reason === null) next.delete(key);
    else next.set(key, reason);
    timeErrors = next;
  }
  const saveBlocked = $derived(
    disk === "conflict"
      ? "Changed on disk: choose Reload or Keep mine."
      : timeErrors.size > 0
        ? `A time: ${[...timeErrors.values()][0]}`
        : docBlocked,
  );

  const autosave = new Autosave<ScheduleDoc>({
    delayMs: SAVE_MS,
    snapshot: () => {
      const d = doc as ScheduleDoc;
      const labels = Object.fromEntries(
        Object.entries(d.labels).map(([k, v]) => [k, v.trim()]),
      ) as unknown as ScheduleLabels;
      return { ...d, name: d.name.trim(), labels, updatedAt: updatedAt ?? "" };
    },
    blocked: () => saveBlocked,
    save: async (d) => {
      const saved = await sourcesApi.saveSchedule(d);
      onsaved(scheduleSummary(saved, usedBy, view));
      return { updatedAt: saved.updatedAt };
    },
    onState: (s) => {
      if (s.kind === "saved" && s.at) updatedAt = s.at;
      if (s.kind !== saveState.kind || s.kind === "saved" || s.kind === "failed") saveState = s;
    },
    describe: (e) => toAppError(e).message,
  });

  function edit(next: ScheduleDoc) {
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

  async function focusItem(itemId: string, key = "title") {
    await tick();
    table?.querySelector<HTMLElement>(`[data-item="${CSS.escape(itemId)}"][data-key="${key}"]`)?.focus();
  }

  // ---- Items --------------------------------------------------------------------------------------

  function onAddItem() {
    if (!doc || !canAddItem(doc)) return;
    const r = addItem(doc, new Date());
    edit(r.doc);
    void focusItem(r.id);
  }

  function onDuplicate(itemId: string) {
    if (!doc) return;
    const r = duplicateItem(doc, itemId);
    if (r.id === "") return;
    edit(r.doc);
    void focusItem(r.id);
  }

  function onDelete(itemId: string) {
    if (!doc) return;
    const i = items.findIndex((x) => x.id === itemId);
    edit(deleteItem(doc, itemId));
    const next = items[Math.min(i, items.length - 1)];
    if (next) void focusItem(next.id);
  }

  // Delegated text inputs: one listener for every row.
  function onTableInput(e: Event) {
    const t = e.target;
    if (!(t instanceof HTMLInputElement) || !doc) return;
    const itemId = t.dataset.item;
    const key = t.dataset.key;
    if (!itemId) return;
    if (key === "title" || key === "subtitle" || key === "location") edit(setText(doc, itemId, key, t.value));
  }

  /** A start or end time from its TimeInput (committed on blur / Enter, so typing never re-sorts). */
  function onTime(itemId: string, key: "time" | "end", value: string) {
    const it = doc?.items.find((x) => x.id === itemId);
    if (!doc || !it) return;
    if (key === "end") {
      edit(setEndTime(doc, it.id, value));
      return;
    }
    const day = validDateTime(it.start) ? datePart(it.start) : localDateTime(new Date()).slice(0, 10);
    edit(setStart(doc, it.id, `${day}T${value}`));
    void focusItem(it.id, "time");
  }

  /** Date / length inputs commit on change (typing a date never re-sorts mid-way). */
  function onTableChange(e: Event) {
    const t = e.target;
    if (!(t instanceof HTMLInputElement) || !doc) return;
    const itemId = t.dataset.item;
    const it = itemId ? doc.items.find((x) => x.id === itemId) : undefined;
    if (!it) return;
    switch (t.dataset.key) {
      case "date": {
        const time = validDateTime(it.start) ? timePart(it.start) : "09:00";
        edit(setStart(doc, it.id, `${t.value}T${time}`));
        void focusItem(it.id, "date");
        break;
      }
      case "length": {
        const n = Number(t.value.trim());
        if (Number.isFinite(n) && n > 0) edit(setDuration(doc, it.id, n));
        else t.value = String(duration(it) || "");
        break;
      }
    }
  }

  function onShift(sign: 1 | -1) {
    if (!doc || doc.items.length === 0 || shiftBy === 0) return;
    const n = sign * shiftBy;
    edit(shiftAll(doc, n));
    toasts.push({
      kind: "info",
      message: `Moved ${itemsLabel(doc.items.length)} ${Math.abs(n)} min ${n > 0 ? "later" : "earlier"}.`,
    });
  }

  function setLabel(key: keyof ScheduleLabels, value: string) {
    if (doc) edit({ ...doc, labels: { ...doc.labels, [key]: value } });
  }

  // ---- Paste / CSV --------------------------------------------------------------------------------

  function addRows(text: string, from: string) {
    if (!doc) return;
    const lines = parseDelimited(text);
    if (lines.length === 0) return;
    if (!canAddItem(doc)) {
      toasts.push({ kind: "error", message: `This schedule already has ${SCHEDULE_ITEMS_MAX} items, the limit.` });
      return;
    }
    const r = applySchedulePaste(doc, lines, localDateTime(new Date()).slice(0, 10));
    if (r.added === 0) {
      toasts.push({
        kind: "error",
        message: `No items in the ${from}: each row needs a title and a start time (HH:MM or a date and time).`,
      });
      return;
    }
    edit(r.doc);
    let message = `Added ${itemsLabel(r.added)}${r.header ? ", first line used as column headers" : ""}.`;
    if (r.skipped > 0) message += ` ${r.skipped} row${r.skipped === 1 ? "" : "s"} without a title or start time left out.`;
    if (r.dropped > 0) message += ` ${r.dropped} over the ${SCHEDULE_ITEMS_MAX}-item limit left out.`;
    toasts.push({ kind: "info", message });
  }

  function onPaste(e: ClipboardEvent) {
    const t = e.target;
    const inTable = t instanceof HTMLInputElement && t.dataset.item !== undefined;
    if ((t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement) && !inTable) return;
    const text = e.clipboardData?.getData("text/plain") ?? "";
    if (!isMultiCell(text) && inTable) return;
    if (text.trim() === "") return;
    e.preventDefault();
    addRows(text, "pasted text");
  }

  async function pasteFromClipboard() {
    let text: string;
    try {
      text = await navigator.clipboard.readText();
    } catch {
      toasts.push({ kind: "error", message: "Couldn't read the clipboard. Click a cell and press Ctrl+V to paste rows." });
      return;
    }
    if (text.trim() === "") {
      toasts.push({ kind: "info", message: "The clipboard has no text to paste." });
      return;
    }
    addRows(text, "clipboard");
  }

  async function onFile(e: Event & { currentTarget: HTMLInputElement }) {
    const input = e.currentTarget;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    if (file.size > 2 * 1024 * 1024) {
      toasts.push({ kind: "error", message: `“${file.name}” is too large (over 2 MB).` });
      return;
    }
    try {
      addRows(await file.text(), "file");
    } catch (err) {
      fail(err);
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

  let confirmMessage = $state<string | null>(null);
  let deleteBusy = $state(false);

  async function askDelete() {
    if (!doc) return;
    const name = doc.name.trim();
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

  // ---- Load, statuses, external changes, teardown -------------------------------------------------

  function applyDoc(d: ScheduleDoc) {
    doc = d;
    updatedAt = d.updatedAt;
    saveState = { kind: "saved", at: d.updatedAt };
  }

  async function reloadFromDisk(): Promise<boolean> {
    let d: ScheduleDoc;
    try {
      d = await sourcesApi.getSchedule(id);
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
    let gotEvent = false;
    const offFiles = on("project-files-changed", onFilesChanged);
    const offView = on("schedule-changed", (v) => {
      if (v.sourceId !== id) return;
      gotEvent = true;
      view = v;
    });
    sourcesApi.getSchedule(id).then(
      (d) => {
        if (!alive) return;
        applyDoc(d);
        loaded = true;
      },
      (e) => {
        if (alive) loadError = toAppError(e).message;
      },
    );
    scheduleView(id).then(
      (v) => {
        if (alive && !gotEvent) view = v;
      },
      () => {
        // No statuses until the next schedule-changed.
      },
    );
    return () => {
      alive = false;
      offFiles();
      offView();
      if (deleted) autosave.cancel();
      else void autosave.flush();
    };
  });

  const savedTitle = $derived(updatedAt ? `Saved ${new Date(updatedAt).toLocaleString()}` : undefined);
</script>

<svelte:window onkeydown={onWindowKey} />

{#if loadError}
  <div class="load-error">
    <p class="bad">Couldn't open this schedule: {loadError}</p>
  </div>
{:else if loaded && doc && issues}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="editor" onpaste={onPaste}>
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

      <span class="muted small">Schedule</span>
      <span class="muted small">{itemsLabel(items.length)}</span>

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

    <div class="toolbar" role="toolbar" aria-label="Schedule">
      <Button
        size="sm"
        disabled={!canAddItem(doc)}
        title={canAddItem(doc) ? undefined : `Up to ${SCHEDULE_ITEMS_MAX} items.`}
        onclick={onAddItem}
      >
        {#snippet icon()}<Plus size={14} />{/snippet}
        Add item
      </Button>
      <Button size="sm" variant="ghost" onclick={() => void pasteFromClipboard()}>
        {#snippet icon()}<ClipboardPaste size={14} />{/snippet}
        Paste rows
      </Button>
      <Button size="sm" variant="ghost" onclick={() => fileInput?.click()}>
        {#snippet icon()}<FileUp size={14} />{/snippet}
        Import CSV…
      </Button>
      <input
        bind:this={fileInput}
        class="visually-hidden"
        type="file"
        accept=".csv,.tsv,.txt,text/csv,text/tab-separated-values,text/plain"
        tabindex="-1"
        aria-hidden="true"
        onchange={onFile}
      />

      <span class="sep" aria-hidden="true"></span>

      <span class="shift" role="group" aria-label="Shift all items">
        <span class="small muted">Shift all by</span>
        <NumberInput
          aria-label="Minutes to shift"
          value={shiftBy}
          min={1}
          max={SHIFT_MAX}
          digits={5}
          suffix="min"
          onchange={(n) => (shiftBy = n)}
        />
        <Button size="sm" variant="ghost" disabled={items.length === 0} onclick={() => onShift(-1)}>Earlier</Button>
        <Button size="sm" variant="ghost" disabled={items.length === 0} onclick={() => onShift(1)}>Later</Button>
      </span>

      <span class="sep" aria-hidden="true"></span>

      <Switch
        label="Ripple delays"
        checked={doc.ripple}
        onchange={(on) => doc && edit({ ...doc, ripple: on })}
      />
      <Button
        size="sm"
        variant="ghost"
        aria-expanded={labelsOpen}
        aria-controls="{uid}-labels"
        onclick={() => (labelsOpen = !labelsOpen)}
      >
        {#snippet icon()}<Tags size={14} />{/snippet}
        Status labels{#if labelIssueCount > 0}<span class="bad">&nbsp;({labelIssueCount})</span>{/if}
      </Button>
    </div>

    {#if labelsOpen || labelIssueCount > 0}
      <div class="labels" id="{uid}-labels" role="group" aria-label="Status labels">
        <p class="small muted">The text shown on air for each status.</p>
        <div class="label-grid">
          {#each LABEL_KEYS as { key, label } (key)}
            {@const err = issues.labels[key]}
            <div class="lab">
              <label class="small muted" for="{uid}-label-{key}">{label}</label>
              <span class="input vf vf-within">
                <input
                  id="{uid}-label-{key}"
                  type="text"
                  autocomplete="off"
                  maxlength={LABEL_MAX + 8}
                  value={doc.labels[key]}
                  aria-invalid={err ? "true" : undefined}
                  aria-describedby={err ? `${uid}-label-${key}-err` : undefined}
                  oninput={(e) => setLabel(key, e.currentTarget.value)}
                />
              </span>
              {#if err}<span class="bad tiny" id="{uid}-label-{key}-err">{err}</span>{/if}
            </div>
          {/each}
        </div>
      </div>
    {/if}

    {#if issues.table}<p class="bad small issue" role="alert">{issues.table}</p>{/if}

    <div class="scroll">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="table"
        role="table"
        aria-label="Items of {doc.name}"
        bind:this={table}
        oninput={onTableInput}
        onchange={onTableChange}
      >
        <div class="hrow" role="row">
          <span role="columnheader">Status</span>
          <span role="columnheader">Date</span>
          <span role="columnheader">Start</span>
          <span role="columnheader">End</span>
          <span role="columnheader">Length</span>
          <span role="columnheader">Delay</span>
          <span role="columnheader">Title</span>
          <span role="columnheader">Subtitle</span>
          <span role="columnheader">Location</span>
          <span role="columnheader"><span class="visually-hidden">Actions</span></span>
        </div>

        {#each groups as g (g.day)}
          <div class="day" role="row">
            <span role="rowheader">{g.day === "" ? "No date" : dayLabel(g.day)}</span>
            <span class="muted small">{itemsLabel(g.items.length)}</span>
          </div>
          {#each g.items as it (it.id)}
            {@const st = statusOf.get(it.id)}
            {@const look = st ? STATUS_LOOK[st.status] : null}
            {@const err = issues.items.get(it.id)}
            {@const len = duration(it)}
            {@const name = it.title.trim() || "item"}
            <div class="row" role="row" data-row={it.id} data-status={st?.status}>
              <span class="cell status-cell" role="cell">
                {#if look && st}
                  <span class="badge" class:strike={look.strike} data-status={st.status}>
                    <TallyDot state={look.tally} pulse={false} />
                    <span>{doc.labels[look.label]}</span>
                  </span>
                {/if}
              </span>
              <span class="cell" role="cell">
                <input
                  type="date"
                  class="mono"
                  data-item={it.id}
                  data-key="date"
                  aria-label="Date of {name}"
                  value={validDateTime(it.start) ? datePart(it.start) : ""}
                  aria-invalid={err?.start ? "true" : undefined}
                  title={err?.start ?? undefined}
                />
              </span>
              <span class="cell" role="cell">
                <TimeInput
                  variant="cell"
                  compact
                  data-item={it.id}
                  data-key="time"
                  aria-label="Start of {name}"
                  value={validDateTime(it.start) ? timePart(it.start) : ""}
                  error={err?.start ?? null}
                  onchange={(v) => onTime(it.id, "time", v)}
                  oninvalid={(r) => setTimeError(`${it.id}/time`, r)}
                />
              </span>
              <span class="cell" role="cell">
                <TimeInput
                  variant="cell"
                  compact
                  data-item={it.id}
                  data-key="end"
                  aria-label="End of {name}"
                  value={validDateTime(it.end) ? timePart(it.end) : ""}
                  error={err?.end ?? null}
                  errorId={err?.end ? `${uid}-${it.id}-end` : undefined}
                  onchange={(v) => onTime(it.id, "end", v)}
                  oninvalid={(r) => setTimeError(`${it.id}/end`, r)}
                />
                {#if validDateTime(it.end) && validDateTime(it.start) && datePart(it.end) !== datePart(it.start)}
                  <span class="tiny muted mono" title="Ends on {dayLabel(datePart(it.end))}">+1d</span>
                {/if}
              </span>
              <span class="cell" role="cell">
                <input
                  type="text"
                  inputmode="numeric"
                  class="mono len"
                  data-item={it.id}
                  data-key="length"
                  aria-label="Length of {name} in minutes"
                  maxlength="5"
                  value={Number.isFinite(len) && len > 0 ? String(len) : ""}
                  title="Minutes, up to {DURATION_MAX}"
                />
                <span class="tiny muted">min</span>
              </span>
              <span class="cell delay mono" role="cell" aria-label="Delay of {name}">
                {#if it.delayMin > 0}
                  <span class="warn">+{it.delayMin}</span>
                  {#if st?.moved}<span class="muted">→ {timePart(st.start)}</span>{/if}
                {:else if st?.moved}
                  <span class="muted">→ {timePart(st.start)}</span>
                {:else}
                  <span class="muted">0</span>
                {/if}
              </span>
              <span class="cell" role="cell">
                <input
                  type="text"
                  autocomplete="off"
                  data-item={it.id}
                  data-key="title"
                  aria-label="Title of the item at {validDateTime(it.start) ? `${dayLabel(datePart(it.start))} ${timePart(it.start)}` : 'no start'}"
                  value={it.title}
                  aria-invalid={err?.title || err?.text ? "true" : undefined}
                  title={err?.title ?? err?.text ?? undefined}
                />
              </span>
              <span class="cell" role="cell">
                <input type="text" autocomplete="off" data-item={it.id} data-key="subtitle" aria-label="Subtitle of {name}" value={it.subtitle} />
              </span>
              <span class="cell" role="cell">
                <input type="text" autocomplete="off" data-item={it.id} data-key="location" aria-label="Location of {name}" value={it.location} />
              </span>
              <span class="cell acts" role="cell">
                <IconButton size="sm" label="Duplicate {name}" tooltip="left" disabled={!canAddItem(doc)} onclick={() => onDuplicate(it.id)}>
                  <Copy size={14} />
                </IconButton>
                <IconButton variant="danger" size="sm" label="Delete {name}" tooltip="left" onclick={() => onDelete(it.id)}>
                  <Trash2 size={14} />
                </IconButton>
              </span>
              {#if err?.end}
                <span class="row-err bad tiny" id="{uid}-{it.id}-end" role="note">{err.end}</span>
              {/if}
            </div>
          {/each}
        {/each}
      </div>

      {#if items.length === 0}
        <div class="empty">
          <p>
            No items yet. Add one, or paste rows from a spreadsheet: title, start, end, location
            (times as <span class="mono">HH:MM</span> or a full date and time).
          </p>
          <span class="empty-acts">
            <Button size="sm" variant="primary" onclick={onAddItem}>
              {#snippet icon()}<Plus size={14} />{/snippet}
              Add item
            </Button>
            <Button size="sm" onclick={() => void pasteFromClipboard()}>
              {#snippet icon()}<ClipboardPaste size={14} />{/snippet}
              Paste rows
            </Button>
          </span>
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if confirmMessage && doc}
  <ConfirmDialog
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

  /* ---- Toolbar ---- */
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--row-h);
    padding: var(--s-2) var(--s-4);
    border-bottom: 1px solid var(--line);
  }
  .sep {
    width: 1px;
    height: var(--s-4);
    margin: 0 var(--s-1);
    background: var(--line);
  }
  .shift {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
  }
  .labels {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3) var(--s-4);
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .label-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr));
    gap: var(--s-2) var(--s-3);
  }
  .lab {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .input {
    display: flex;
    border-radius: var(--r-control);
  }
  .input input {
    width: 100%;
    min-width: 0;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .input input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .issue {
    padding: var(--s-2) var(--s-4);
    border-bottom: 1px solid var(--line);
  }

  /* ---- Table ---- */
  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .table {
    /* Times first (what an operator scans), then the text columns, which share the rest. */
    --cols: 7.5rem 8.5rem 6rem 7.5rem 4.75rem 7rem minmax(8rem, 2fr) minmax(7rem, 1.2fr) minmax(6rem, 1fr) 4.5rem;
    min-width: max-content;
  }
  .hrow,
  .row {
    display: grid;
    grid-template-columns: var(--cols);
    min-width: 100%;
  }
  .hrow {
    position: sticky;
    top: 0;
    z-index: 1;
    min-height: var(--row-h);
    align-items: center;
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
  }
  .hrow > span {
    padding: 0 var(--s-2);
  }
  .day {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-3) var(--s-1);
    border-bottom: 1px solid var(--line);
    font-weight: 600;
  }
  .row {
    content-visibility: auto;
    contain-intrinsic-size: auto var(--row-h);
    border-bottom: 1px solid var(--line);
  }
  .row[data-status="now"] {
    background: var(--surface-1);
  }
  /* :global reaches the start / end TimeInputs too. */
  .row[data-status="previous"] :global(input),
  .row[data-status="cancelled"] :global(input) {
    color: var(--text-muted);
  }
  .cell {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
    border-left: 1px solid var(--line);
  }
  .cell:first-child {
    border-left: 0;
  }
  .cell input {
    width: 100%;
    min-width: 0;
    height: var(--row-h);
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
  }
  .cell input:hover {
    background: var(--surface-2);
  }
  .cell input:focus-visible,
  .cell input:focus {
    outline: none;
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--focus);
  }
  .cell input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .cell .len {
    width: 3.5rem;
    flex: none;
    text-align: right;
  }
  .status-cell {
    padding: 0 var(--s-2);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    max-width: 100%;
    overflow: hidden;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
    white-space: nowrap;
  }
  .badge[data-status="previous"],
  .badge[data-status="cancelled"],
  .badge[data-status="upcoming"] {
    color: var(--text-muted);
  }
  .badge[data-status="delayed"] {
    color: var(--warning);
  }
  .badge.strike span:last-child {
    text-decoration: line-through;
  }
  .delay {
    padding: 0 var(--s-2);
    font-size: var(--fs-sm);
  }
  .warn {
    color: var(--warning);
  }
  .acts {
    justify-content: flex-end;
    padding: 0 var(--s-1);
  }
  .row-err {
    grid-column: 1 / -1;
    padding: 0 var(--s-2) var(--s-1) calc(8.5rem + var(--s-2));
  }

  .empty {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-5) var(--s-4);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .empty-acts {
    display: inline-flex;
    gap: var(--s-2);
  }

  .small {
    font-size: var(--fs-sm);
  }
  .tiny {
    font-size: var(--fs-xs);
  }
  .muted {
    color: var(--text-muted);
  }
  .bad {
    color: var(--danger);
  }
  .load-error {
    padding: var(--s-6);
  }
</style>
