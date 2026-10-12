<!--
  LayersPanel: the screen editor's left column. The layers top = front (the reverse of the doc
  order), each row with its on-air tally, name, template name, "Changes on next take" and
  "Template missing" / "Source missing" markers, Play in / Play out and Delete.

  Right-click a row (or Shift+F10 / the menu key on it): the lazy context menu with Assign MIDI…
  (Take or clear) and Remove MIDI.

  Reorder: drag a row by its grip (pointer events; only the dragged row moves, by transform), or
  Alt+ArrowUp / Alt+ArrowDown on a focused row. ArrowUp/ArrowDown move focus between rows, Delete
  removes the focused row's layer.

  Props:
    screenId: string               the screen (for the context menu's MIDI target)
    rows: LayerRow[]               panel order (top = front)
    selectedIds: string[]          the selection (Shift/Ctrl+click a row adds or removes it)
    templates: TemplateSummary[]   for the Add layer menu
    canAdd: boolean
    busyIds: string[]              layers with a Play request in flight
    onselect: (id: string, additive: boolean) => void
    onshift: (id: string, dir: 1 | -1) => void        +1 = towards the front (up in the list)
    onreorder: (id: string, toRow: number) => void    drop at panel row index
    ondelete: (id: string) => void
    onplay: (id: string, show: boolean) => void
    onadd: (templateId: string) => void
    onnewtemplate: () => void
-->
<script lang="ts" module>
  // The Add layer menu is its own chunk, requested when the panel first mounts (it appears a
  // moment after the list), keeping the editor route's initial JS small.
  const addLayerMenu = import("./AddLayerMenu.svelte");

  export interface LayerRow {
    id: string;
    name: string;
    templateName: string | null;
    /** A media layer (`templateName` is then its `/assets/<name>` value). */
    media?: boolean;
    onAir: boolean;
    pending: boolean;
    missing: boolean;
    /** A bound source, column or row is missing (the layer shows its own values). */
    sourceMissing?: boolean;
  }
</script>

<script lang="ts">
  import { tick } from "svelte";
  import GripVertical from "lucide-svelte/icons/grip-vertical";
  import Play from "lucide-svelte/icons/play";
  import Square from "lucide-svelte/icons/square";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { assetName } from "$lib/components/assets/assets";
  import { contextMenu, isMenuKey } from "$lib/components/midi/menu";
  import { SCREEN_LAYERS_MAX, type TemplateSummary } from "$lib/types";

  let {
    screenId,
    rows,
    selectedIds,
    templates,
    canAdd,
    busyIds = [],
    onselect,
    onshift,
    onreorder,
    ondelete,
    onplay,
    onadd,
    onnewtemplate,
    onmedia,
  }: {
    screenId: string;
    rows: readonly LayerRow[];
    selectedIds: readonly string[];
    templates: readonly TemplateSummary[];
    canAdd: boolean;
    busyIds?: readonly string[];
    onselect: (id: string, additive: boolean) => void;
    onshift: (id: string, dir: 1 | -1) => void;
    onreorder: (id: string, toRow: number) => void;
    ondelete: (id: string) => void;
    onplay: (id: string, show: boolean) => void;
    onadd: (templateId: string) => void;
    onnewtemplate: () => void;
    onmedia?: () => void;
  } = $props();

  const uid = $props.id();
  let list: HTMLUListElement | undefined = $state();

  async function focusRow(id: string) {
    await tick();
    list?.querySelector<HTMLButtonElement>(`[data-layer="${CSS.escape(id)}"] .main`)?.focus();
  }

  const menu = (e: MouseEvent | KeyboardEvent, row: LayerRow) =>
    contextMenu(e, { target: { kind: "layer", screenId, layerId: row.id }, name: row.name });

  function onRowKey(e: KeyboardEvent, row: LayerRow, i: number) {
    if (isMenuKey(e)) return menu(e, row);
    const t = e.target as HTMLElement;
    if (t.closest("input, textarea, select")) return;
    if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      onshift(row.id, e.key === "ArrowUp" ? 1 : -1);
      void focusRow(row.id);
    } else if (!e.altKey && !e.ctrlKey && !e.metaKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      const next = rows[i + (e.key === "ArrowUp" ? -1 : 1)];
      if (!next) return;
      e.preventDefault();
      void focusRow(next.id);
    } else if (e.key === "Delete" && !e.altKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      const neighbour = rows[i + 1] ?? rows[i - 1];
      ondelete(row.id);
      if (neighbour) void focusRow(neighbour.id);
    }
  }

  // ---- Drag reorder ---------------------------------------------------------------------------------

  let reorder = $state<{ id: string; from: number; to: number; dy: number } | null>(null);
  let mids: number[] = [];
  let startY = 0;

  function onGripDown(e: PointerEvent, row: LayerRow, i: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    const items = [...(list?.querySelectorAll<HTMLElement>(":scope > li") ?? [])];
    mids = items.map((el) => {
      const r = el.getBoundingClientRect();
      return r.top + r.height / 2;
    });
    startY = e.clientY;
    try {
      (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    } catch {
      // Synthetic pointers can't be captured.
    }
    reorder = { id: row.id, from: i, to: i, dy: 0 };
  }

  function onGripMove(e: PointerEvent) {
    if (!reorder) return;
    const y = e.clientY;
    let to = mids.findIndex((m) => y < m);
    if (to < 0) to = mids.length - 1;
    else if (to > reorder.from) to -= 1;
    reorder = { ...reorder, to: Math.max(0, Math.min(rows.length - 1, to)), dy: y - startY };
  }

  function onGripUp() {
    if (!reorder) return;
    const r = reorder;
    reorder = null;
    if (r.to !== r.from) {
      onreorder(r.id, r.to);
      void focusRow(r.id);
    }
  }
</script>

<section class="panel" aria-labelledby="{uid}-title">
  <header class="head">
    <h2 class="section-label" id="{uid}-title">Layers</h2>
    <span class="count">{rows.length} / {SCREEN_LAYERS_MAX}</span>
    <span class="spacer"></span>
    {#await addLayerMenu then { default: AddLayerMenu }}
      <AddLayerMenu
        {templates}
        disabled={!canAdd}
        disabledReason="Up to {SCREEN_LAYERS_MAX} layers per screen."
        onpick={onadd}
        {onnewtemplate}
        {onmedia}
      />
    {/await}
  </header>

  {#if rows.length === 0}
    <p class="empty">Add a layer to place a template on this screen.</p>
  {:else}
    <ul class="rows" bind:this={list} aria-label="Layers, front to back">
      {#each rows as row, i (row.id)}
        {@const selected = selectedIds.includes(row.id)}
        {@const busy = busyIds.includes(row.id)}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <li
          class="row vf vf-within"
          class:lifted={reorder?.id === row.id}
          class:drop-before={reorder !== null && reorder.id !== row.id && reorder.to === i && reorder.to < reorder.from}
          class:drop-after={reorder !== null && reorder.id !== row.id && reorder.to === i && reorder.to > reorder.from}
          style:transform={reorder?.id === row.id ? `translateY(${reorder.dy}px)` : undefined}
          data-selected={selected ? "" : undefined}
          data-layer={row.id}
          onkeydown={(e) => onRowKey(e, row, i)}
          oncontextmenu={(e) => menu(e, row)}
        >
          <button
            type="button"
            class="grip"
            aria-label="Reorder {row.name} (Alt+Up / Alt+Down)"
            tabindex="-1"
            onpointerdown={(e) => onGripDown(e, row, i)}
            onpointermove={onGripMove}
            onpointerup={onGripUp}
            onpointercancel={onGripUp}
          >
            <GripVertical size={14} />
          </button>

          <button type="button" class="main" aria-pressed={selected} onclick={(e) => onselect(row.id, e.shiftKey || e.ctrlKey || e.metaKey)}>
            <TallyDot state={row.onAir ? "program" : "idle"} label={row.onAir ? "On air" : "Off air"} />
            <span class="names">
              <span class="name">{row.name}</span>
              <span class="tpl" class:mono={row.media}>{row.media ? assetName(row.templateName ?? "") : (row.templateName ?? "Unknown template")}</span>
            </span>
          </button>

          <span class="acts">
            {#if row.onAir}
              <Button size="sm" disabled={busy} onclick={() => onplay(row.id, false)} aria-label="Play out {row.name}">
                {#snippet icon()}<Square size={12} />{/snippet}
                Out
              </Button>
            {:else}
              <Button
                size="sm"
                variant="primary"
                disabled={busy || row.missing}
                onclick={() => onplay(row.id, true)}
                aria-label="Play in {row.name}"
              >
                {#snippet icon()}<Play size={12} />{/snippet}
                In
              </Button>
            {/if}
            <IconButton size="sm" variant="danger" label="Delete layer" tooltip="left" onclick={() => ondelete(row.id)}>
              <Trash2 size={14} />
            </IconButton>
          </span>

          {#if row.pending || row.missing || row.sourceMissing}
            <span class="markers">
              {#if row.pending}<span class="marker pending">Changes on next take</span>{/if}
              {#if row.missing}<span class="marker missing">Template missing</span>{/if}
              {#if row.sourceMissing}<span class="marker missing">Source missing</span>{/if}
            </span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface-1);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: calc(var(--row-h) + var(--s-2));
    padding: var(--s-1) var(--s-3);
    border-bottom: 1px solid var(--line);
  }
  .count {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .spacer {
    flex: 1;
  }
  .empty {
    padding: var(--s-4) var(--s-3);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }

  .rows {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
    margin: 0;
    padding: var(--s-2);
    overflow: auto;
    list-style: none;
  }
  .row {
    --vf-radius: var(--r-control);
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 0 var(--s-1);
    padding: var(--s-1) var(--s-1) var(--s-1) 0;
    border-radius: var(--r-control);
    transition: transform var(--dur-fast) var(--ease);
  }
  .row:hover {
    background: var(--surface-2);
  }
  .row[data-selected] {
    background: var(--selected-bg);
  }
  .row.lifted {
    z-index: 1;
    transition: none;
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .row.drop-before::before,
  .row.drop-after::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    border-radius: 1px;
    background: var(--accent);
    pointer-events: none;
  }
  .row.drop-before::before {
    top: -2px;
  }
  .row.drop-after::before {
    bottom: -2px;
  }

  .grip {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--s-5);
    height: var(--row-h);
    color: var(--text-muted);
    cursor: grab;
    touch-action: none;
  }
  .grip:hover {
    color: var(--text);
  }
  .main {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    min-height: var(--row-h);
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    text-align: left;
  }
  .names {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.25;
  }
  .name,
  .tpl {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tpl {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .acts {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .markers {
    grid-column: 2 / -1;
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-1) var(--s-2);
    padding: 0 var(--s-1) var(--s-1);
  }
  .marker {
    font-size: var(--fs-xs);
  }
  .marker.pending {
    color: var(--accent-text);
  }
  .marker.missing {
    color: var(--warning);
  }
  @media (prefers-reduced-motion: reduce) {
    .row {
      transition: none;
    }
  }
</style>
