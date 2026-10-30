<!--
  ListDrawer: a wide drawer (about 80% of the window, right-anchored) over Control or the screen
  editor, hosting the spreadsheet editor (SheetEditor, its own lazy chunk) for one list field. A
  lazy chunk itself: ListField imports it on the first Edit.

  A native modal <dialog>: focus is trapped and the page behind is inert; Esc (when the sheet
  didn't use it, e.g. to cancel a cell edit), × or a backdrop click closes it; focus returns to
  where it was (the summary's Edit button).

  Two targets (./drawer.ts):
    list    the layer's own rows. Every sheet change goes to `onchange` as the list value; the
            host applies it (Control: setLayerFields, debounced 150 ms, so it follows Studio mode
            and Live edit server-side). Limits: LIST_ROWS_MAX rows, 64 KB.
    source  the data table the list is linked to: loaded with getSource, saved like the Sources
            page (autosave 500 ms after the last change, flushed on close). Columns are edited on
            the Sources page only. "Changes affect N layers." counts the layers bound to it.

  Props:
    target: ListDrawerTarget
    opener?: HTMLElement | null  where focus returns on close (default: what had focus on open)
    onclose: () => void        after the drawer closed (pending saves are flushed first)
-->
<script module lang="ts">
  let sheetChunk: Promise<typeof import("$lib/components/sheet/SheetEditor.svelte")> | null = null;
  const loadSheet = () => (sheetChunk ??= import("$lib/components/sheet/SheetEditor.svelte"));
</script>

<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import { tick, untrack } from "svelte";
  import X from "lucide-svelte/icons/x";
  import { toAppError } from "$lib/api";
  import { getScreen } from "$lib/api/screens";
  import { getSource, saveSource } from "$lib/api/sources";
  import IconButton from "$lib/components/IconButton.svelte";
  import type { SheetRow } from "$lib/components/sheet/types";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { SourceColumn, SourceDoc } from "$lib/types";
  import {
    LIST_LIMITS,
    SOURCE_LIMITS,
    SourceSaver,
    affectsNote,
    followIndex,
    layersUsing,
    type ListDrawerTarget,
  } from "./drawer";
  import { parseRows, serializeRows } from "./list";

  let {
    target,
    opener = null,
    onclose,
  }: { target: ListDrawerTarget; opener?: HTMLElement | null; onclose: () => void } = $props();

  const SAVE_MS = 500;
  // Kept from the start: a save can finish after the drawer closed (flushed on close).
  // svelte-ignore state_referenced_locally
  const onsaved = target.kind === "source" ? target.onsaved : undefined;
  const uid = $props.id();

  let dialog: HTMLDialogElement | undefined = $state();
  let returnFocus: HTMLElement | null = null;
  let closed = false;

  // ---- What the sheet shows ---------------------------------------------------------------------

  /** The working rows of a list target (taken once: the drawer owns them while open). */
  // svelte-ignore state_referenced_locally
  const listRows: SheetRow[] = target.kind === "list" ? parseRows(target.value) : [];

  /** The data table (source target), once loaded. */
  let doc = $state.raw<SourceDoc | null>(null);
  let loadError = $state<string | null>(null);
  let layers = $state<number | null>(null);
  let saveState = $state<"pending" | "saving" | "saved" | "failed">("saved");

  const title = $derived(target.kind === "list" ? target.label : target.name);
  const note = $derived(target.kind === "source" && layers !== null ? affectsNote(layers) : null);

  const autosave = new SourceSaver<SourceDoc>(
    SAVE_MS,
    () => doc as SourceDoc,
    async (d) => {
      const saved = await saveSource(d);
      if (doc) doc = { ...doc, updatedAt: saved.updatedAt };
      onsaved?.();
    },
    (s, e) => {
      saveState = s;
      if (s === "failed") toasts.push({ kind: "error", message: `Couldn't save “${title}”: ${toAppError(e).message}` });
    },
  );

  async function loadSource(t: Extract<ListDrawerTarget, { kind: "source" }>) {
    try {
      doc = await getSource(t.sourceId);
    } catch (e) {
      loadError = toAppError(e).message;
      return;
    }
    // The note: layers bound to the table on every screen that uses it (best effort).
    const docs = await Promise.all(t.usedBy.map((id) => getScreen(id).then((s) => s.doc, () => null)));
    if (!closed) layers = layersUsing(docs.filter((d) => d !== null), t.sourceId);
  }

  function onsheet(rows: SheetRow[], _columns: SourceColumn[], origin: readonly number[]) {
    if (target.kind === "list") {
      target.onchange(serializeRows(rows, target.columns));
      return;
    }
    if (!doc) return;
    doc = { ...doc, rows, index: followIndex(doc.index, origin) };
    autosave.touch();
  }

  // ---- Open / close -----------------------------------------------------------------------------

  $effect(() => {
    const d = dialog;
    if (!d) return;
    // Once, when the dialog exists (the target and opener are fixed while open).
    untrack(() => {
      returnFocus = opener ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
      if (target.kind === "source") void loadSource(target);
    });
    return () => {
      // Unmounted without close() (the host went away): still save what was typed.
      void autosave.flush();
    };
  });

  async function close() {
    if (closed) return;
    closed = true;
    const d = dialog;
    if (d?.open) {
      if (typeof d.close === "function") d.close();
      else d.removeAttribute("open");
    }
    const el = returnFocus;
    returnFocus = null;
    await tick();
    if (el?.isConnected) el.focus();
    void autosave.flush();
    onclose();
  }

  // Esc: the sheet handles it first (cancelling a cell edit prevents the default); otherwise it
  // closes. The native cancel never closes on its own, so both paths end in close().
  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && !e.defaultPrevented) {
      e.preventDefault();
      void close();
    }
  }

  function oncancel(e: Event) {
    e.preventDefault();
  }

</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events -->
<dialog bind:this={dialog} class="drawer" aria-labelledby="{uid}-title" {oncancel} {@attach backdropClose(() => void close())} {onkeydown}>
  <div class="inner">
    <header class="head">
      <div class="titles">
        <h2 class="title" id="{uid}-title">
          {#if target.kind === "source"}<span class="kind">Data table</span>{/if}
          {title}
        </h2>
        {#if note}<p class="note">{note}</p>{/if}
      </div>
      {#if target.kind === "source"}
        <span class="state" role="status">
          {#if saveState === "pending" || saveState === "saving"}Saving…{:else if saveState === "failed"}Not saved{/if}
        </span>
      {/if}
      <IconButton label="Close" tooltip="left" onclick={() => void close()}>
        <X size={16} />
      </IconButton>
    </header>
    <div class="body">
      {#if target.kind === "list"}
        {#await loadSheet() then { default: SheetEditor }}
          <SheetEditor
            {title}
            hideTitle
            columns={[...target.columns]}
            rows={listRows}
            limits={LIST_LIMITS}
            onchange={onsheet}
            onclose={() => void close()}
          />
        {/await}
      {:else if loadError}
        <p class="error" role="alert">Couldn't load the data table: {loadError}</p>
      {:else if doc}
        {@const d = doc}
        {#await loadSheet() then { default: SheetEditor }}
          <SheetEditor
            {title}
            hideTitle
            columns={d.columns}
            rows={d.rows}
            limits={SOURCE_LIMITS}
            currentRow={d.index}
            onchange={onsheet}
            onclose={() => void close()}
          />
        {/await}
      {/if}
    </div>
  </div>
</dialog>

<style>
  .drawer {
    width: 80vw;
    min-width: min(100vw, 40rem);
    max-width: none;
    height: 100dvh;
    max-height: none;
    margin: 0 0 0 auto;
    padding: 0;
    border: 0;
    border-left: 1px solid var(--line);
    background: var(--surface-1);
    color: var(--text);
    overflow: hidden;
  }
  .drawer[open] {
    animation: slide var(--dur-slow) var(--ease);
  }
  .drawer::backdrop {
    background: var(--overlay);
    animation: fade var(--dur) var(--ease);
  }
  .inner {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-height: calc(var(--row-h) + 2 * var(--s-2));
    padding: var(--s-2) var(--s-2) var(--s-2) var(--s-4);
    border-bottom: 1px solid var(--line);
  }
  .titles {
    flex: 1;
    min-width: 0;
  }
  .title {
    margin: 0;
    overflow: hidden;
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind {
    margin-right: var(--s-2);
    color: var(--text-muted);
    font-family: var(--font-body);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .note {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .state {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .body {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }
  .error {
    margin: 0;
    padding: var(--s-4);
    color: var(--danger);
  }
  /* Slides only, never fades: opaque from its first frame (see the dialogs' enter). */
  @keyframes slide {
    from {
      transform: translateX(24px);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .drawer[open],
    .drawer::backdrop {
      animation: none;
    }
  }
</style>
