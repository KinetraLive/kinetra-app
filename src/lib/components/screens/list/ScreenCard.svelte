<!--
  One screen in the Screens list: a live thumbnail of the REAL /screen/<id> URL (fitted into a 16:9
  box over the transparency checkerboard), a tally dot + name, the mono URL with a copy button, the
  resolution and layer count in mono, and the actions (Open, Duplicate, Rename, Delete).

  A selectable object: the name button covers the whole card (click selects, Enter or double-click
  opens /screens/<id>). Viewfinder brackets while `selected` and on keyboard focus anywhere in the
  card. Roving tab stop: only the selected card's controls are in the tab order; the grid moves the
  selection with the arrow keys. Rename edits the name in place (Enter/blur saves, Esc cancels).

  The grid (ScreenGrid) decides whether the iframe exists (`live`); this component never creates
  one on its own.

  Props:
    screen: ScreenSummary
    url: string | null       local screen URL (null while the server isn't running)
    lanUrl: string | null    LAN screen URL when LAN access is on; shown and copied instead
    tally: ScreenTally
    live: boolean            mount the preview iframe
    note: string | null      muted line over the checkerboard when not live ("Preview paused")
    box: { w, h }            thumbnail box size in px (for the scale)
    selected: boolean
    busy?: boolean           a mutation is in flight (disables the actions)
    track: Attachment        observer registration for the thumbnail box (from the grid)
    onselect, onopen, onduplicate, ondelete: () => void
    onrename: (name: string) => Promise<boolean>   resolves true when the rename was saved
-->
<script lang="ts">
  import type { Attachment } from "svelte/attachments";
  import ArrowUpRight from "lucide-svelte/icons/arrow-up-right";
  import Copy from "lucide-svelte/icons/copy";
  import CopyPlus from "lucide-svelte/icons/copy-plus";
  import Pencil from "lucide-svelte/icons/pencil";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import IconButton from "$lib/components/IconButton.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { copyUrl } from "$lib/stores/server.svelte";
  import { SCREEN_NAME_MAX, type ScreenSummary } from "$lib/types";
  import { validScreenName } from "../new/format";
  import {
    TALLY_LABELS,
    fitScale,
    layersLabel,
    resolutionLabel,
    type ScreenTally,
  } from "./format";

  type Props = {
    screen: ScreenSummary;
    url: string | null;
    lanUrl: string | null;
    tally: ScreenTally;
    live: boolean;
    note: string | null;
    box: { w: number; h: number };
    selected: boolean;
    busy?: boolean;
    track: Attachment<HTMLElement>;
    onselect: () => void;
    onopen: () => void;
    onduplicate: () => void;
    ondelete: () => void;
    onrename: (name: string) => Promise<boolean>;
  };
  let {
    screen,
    url,
    lanUrl,
    tally,
    live,
    note,
    box,
    selected,
    busy = false,
    track,
    onselect,
    onopen,
    onduplicate,
    ondelete,
    onrename,
  }: Props = $props();

  const uid = $props.id();
  const nativeW = $derived(screen.width || 1920);
  const nativeH = $derived(screen.height || 1080);
  const scale = $derived(fitScale(box.w, box.h, nativeW, nativeH));
  const shown = $derived(lanUrl ?? url);
  const tab = $derived(selected ? 0 : -1);
  const fill = $derived(screen.background === "transparent" ? undefined : screen.background);

  let editing = $state(false);
  let draft = $state("");
  let saving = $state(false);
  let pick: HTMLButtonElement | undefined = $state();

  function startRename() {
    draft = screen.name;
    editing = true;
  }

  const focusSelect = (el: HTMLInputElement) => {
    el.focus();
    el.select();
  };

  function stop() {
    editing = false;
    queueMicrotask(() => pick?.focus());
  }

  async function commit(e?: SubmitEvent) {
    e?.preventDefault();
    if (saving || !editing) return;
    const name = draft.trim();
    if (name === screen.name || !validScreenName(name)) {
      stop();
      return;
    }
    saving = true;
    try {
      if (await onrename(name)) stop();
    } finally {
      saving = false;
    }
  }

  function onInputKeydown(e: KeyboardEvent) {
    // Keys typed into the name never reach the grid (arrows, Delete).
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      stop();
    }
  }

  function onPickKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      onopen();
    } else if (e.key === "F2") {
      e.preventDefault();
      startRename();
    }
  }
</script>

<article
  class="card vf vf-live"
  data-tally={tally === "program" ? "program" : undefined}
  data-selected={selected || undefined}
  data-screen={screen.id}
  aria-labelledby="{uid}-name"
>
  <div class="thumb" {@attach track}>
    <div
      class="stage checker"
      style:width="{nativeW * scale}px"
      style:height="{nativeH * scale}px"
      style:background={fill}
    >
      {#if live && url}
        <iframe
          src="{url}?mute=1"
          title="{screen.name} preview"
          tabindex="-1"
          sandbox="allow-scripts allow-same-origin"
          style:width="{nativeW}px"
          style:height="{nativeH}px"
          style:transform="scale({scale})"
        ></iframe>
      {/if}
    </div>
    {#if !live && note}
      <span class="paused">{note}</span>
    {/if}
  </div>

  <div class="foot">
    <div class="title">
      <TallyDot state={tally} label={TALLY_LABELS[tally]} />
      {#if editing}
        <form class="rename" onsubmit={commit}>
          <label class="visually-hidden" for="{uid}-input">Screen name</label>
          <div class="input vf vf-within">
            <input
              id="{uid}-input"
              type="text"
              bind:value={draft}
              {@attach focusSelect}
              maxlength={SCREEN_NAME_MAX}
              required
              autocomplete="off"
              spellcheck="false"
              disabled={saving}
              onkeydown={onInputKeydown}
              onblur={() => void commit()}
            />
          </div>
        </form>
        <span id="{uid}-name" class="visually-hidden">{screen.name}</span>
      {:else}
        <button
          bind:this={pick}
          id="{uid}-name"
          type="button"
          class="pick"
          data-pick
          aria-current={selected || undefined}
          tabindex={tab}
          onclick={onselect}
          ondblclick={onopen}
          onkeydown={onPickKeydown}>{screen.name}</button
        >
      {/if}
    </div>

    <div class="row">
      <span class="url mono" title={shown ?? undefined}>{shown ?? `/screen/${screen.id}`}</span>
      <IconButton
        label="Copy URL of {screen.name}"
        size="sm"
        tooltip="left"
        tabindex={tab}
        disabled={!shown}
        onclick={() => shown && copyUrl(shown)}
      >
        <Copy size={14} />
      </IconButton>
    </div>

    <div class="row">
      <span class="meta"
        ><span class="mono">{resolutionLabel({ width: nativeW, height: nativeH })}</span> · {layersLabel(screen.layerCount)}</span
      >
      <span class="actions">
        <IconButton label="Open {screen.name}" size="sm" tooltip="top" tabindex={tab} onclick={onopen}>
          <ArrowUpRight size={14} />
        </IconButton>
        <IconButton
          label="Duplicate {screen.name}"
          size="sm"
          tooltip="top"
          tabindex={tab}
          disabled={busy}
          onclick={onduplicate}
        >
          <CopyPlus size={14} />
        </IconButton>
        <IconButton
          label="Rename {screen.name}"
          size="sm"
          tooltip="top"
          tabindex={tab}
          disabled={busy || editing}
          onclick={startRename}
        >
          <Pencil size={14} />
        </IconButton>
        <IconButton
          label="Delete {screen.name}"
          variant="danger"
          size="sm"
          tooltip="top"
          tabindex={tab}
          disabled={busy}
          onclick={ondelete}
        >
          <Trash2 size={14} />
        </IconButton>
      </span>
    </div>
  </div>
</article>

<style>
  .card {
    --vf-radius: var(--r-panel);
    display: flex;
    flex-direction: column;
    min-width: 0;
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .card:hover {
    background: var(--surface-2);
  }
  .card[data-selected] {
    background: var(--surface-2);
    /* Just outside the edge (the thumbnail would cover a line inside); on air, the tally line runs
       right outside it. */
    outline: 1px solid var(--selected-line);
    outline-offset: 0;
  }
  /* Keyboard focus anywhere in the card draws the card's focus ring. */
  .card:has(:focus-visible)::after {
    opacity: 1;
  }
  .pick:focus-visible {
    outline: none;
  }

  .thumb {
    position: relative;
    display: grid;
    place-items: center;
    aspect-ratio: 16 / 9;
    overflow: hidden;
    border-radius: var(--r-panel) var(--r-panel) 0 0;
    border-bottom: 1px solid var(--line);
    background: var(--bg);
  }
  .stage {
    position: relative;
    overflow: hidden;
  }
  .stage iframe {
    position: absolute;
    top: 0;
    left: 0;
    border: 0;
    background: transparent;
    /* The renderer page uses the default (light) colour scheme. When an iframe's scheme differs
       from its embedder's, the browser paints it with an opaque canvas, so in the dark app theme
       the transparent output would show as solid white. Match the renderer's scheme. */
    color-scheme: light;
    transform-origin: 0 0;
    pointer-events: none;
  }
  .paused {
    position: absolute;
    max-width: calc(100% - 2 * var(--s-3));
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-1);
    color: var(--text-muted);
    font-size: var(--fs-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .foot {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--s-2) var(--s-2) var(--s-2) var(--s-3);
  }
  .title {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    height: var(--control-h);
  }
  .pick {
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font-size: var(--fs-md);
    font-weight: 600;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
    user-select: none;
  }
  /* The whole card is the name button's hit area. */
  .pick::before {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: var(--r-panel);
  }
  .rename {
    flex: 1;
    min-width: 0;
  }
  .input {
    --vf-radius: var(--r-control);
    display: flex;
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .input input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
  }
  .row {
    /* Above the name button's hit area, so the buttons and URL selection work. */
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
  }
  .url,
  .meta {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .url {
    font-size: var(--fs-sm);
    user-select: all;
  }
  .meta {
    font-size: var(--fs-xs);
  }
  .actions {
    display: flex;
    flex: none;
    gap: 2px;
  }
</style>
