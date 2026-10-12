<!--
  ControlTile: one screen in the Control grid. Live thumbnail of the REAL /screen/<id> (scaled from
  the native size over the checkerboard), on-air tally with text, name, mono "n on air" and Clear.
  A selectable object: the whole tile is the select button's hit area (click / Enter / Space);
  selected: a neutral line (--selected-line) on --surface-2; on air: a solid red line outside the
  edge; Studio Preview pending: a dashed green one (data-tally, app.css); keyboard focus: the focus
  ring. Clear sits above the hit area.

  The grid decides whether the iframe exists (`live`); the tile never creates one on its own.

  Props:
    view: LiveScreenView
    url: string | null        screen URL (null while the server isn't running)
    live: boolean             mount the preview iframe
    note: string | null       muted line over the checkerboard when not live
    width: number             thumbnail box width in px (for the scale)
    selected: boolean
    pending?: boolean         Studio mode: the Preview differs from Program ("Preview pending")
    track: Attachment         observer registration for the thumbnail box (from the grid)
    onselect: () => void
    onclear: () => void
-->
<script lang="ts">
  import type { Attachment } from "svelte/attachments";
  import Button from "$lib/components/Button.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import type { LiveScreenView } from "$lib/types";
  import { onAirCount } from "./control.svelte";

  let {
    view,
    url,
    live,
    note,
    width,
    selected,
    pending = false,
    track,
    onselect,
    onclear,
  }: {
    view: LiveScreenView;
    url: string | null;
    live: boolean;
    note: string | null;
    width: number;
    selected: boolean;
    pending?: boolean;
    track: Attachment<HTMLElement>;
    onselect: () => void;
    onclear: () => void;
  } = $props();

  const info = $derived(view.screen);
  const nativeW = $derived(info.width || 1920);
  const nativeH = $derived(info.height || 1080);
  const scale = $derived(width > 0 ? width / nativeW : 0);
  const count = $derived(onAirCount(view));
</script>

<article
  class="tile vf vf-live"
  data-selected={selected ? "" : undefined}
  data-tally={count > 0 ? "program" : pending ? "preview" : undefined}
  data-screen={info.id}
>
  <div class="thumb checker" style:aspect-ratio="{nativeW} / {nativeH}" {@attach track}>
    {#if live && url}
      <iframe
        src="{url}?mute=1"
        title="{info.name} preview"
        tabindex="-1"
        sandbox="allow-scripts allow-same-origin"
        style:width="{nativeW}px"
        style:height="{nativeH}px"
        style:transform="scale({scale})"
      ></iframe>
    {:else if note}
      <span class="note">{note}</span>
    {/if}
  </div>
  <div class="foot">
    <TallyDot state={count > 0 ? "program" : "idle"} label={count > 0 ? "On air" : "Off air"} />
    <button type="button" class="pick" aria-pressed={selected} onclick={onselect}>{info.name}</button>
    {#if pending}<span class="pending"><TallyDot state="preview" size={6} />Preview pending</span>{/if}
    <span class="count">{count} on air</span>
    <span class="clear">
      <Button size="sm" variant="ghost" disabled={count === 0} onclick={onclear} aria-label="Clear {info.name}">
        Clear
      </Button>
    </span>
  </div>
</article>

<style>
  .tile {
    --vf-radius: var(--r-panel);
    display: flex;
    flex-direction: column;
    min-width: 0;
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .tile:hover {
    background: var(--surface-2);
  }
  .tile[data-selected] {
    background: var(--surface-2);
    /* Just outside the edge (the thumbnail would cover a line inside); on air, the tally line runs
       right outside it. */
    outline: 1px solid var(--selected-line);
    outline-offset: 0;
  }
  /* Keyboard focus on the select button draws the tile's focus ring. */
  .tile:has(.pick:focus-visible)::after {
    opacity: 1;
  }
  .pick:focus-visible {
    outline: none;
  }
  .thumb {
    position: relative;
    overflow: hidden;
    border-radius: var(--r-panel) var(--r-panel) 0 0;
    border-bottom: 1px solid var(--line);
  }
  iframe {
    position: absolute;
    top: 0;
    left: 0;
    border: 0;
    background: transparent;
    /* Match the renderer's scheme, else the transparent output paints white in the dark theme. */
    color-scheme: light;
    transform-origin: 0 0;
    pointer-events: none;
  }
  .note {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    min-height: var(--row-h);
    padding: var(--s-1) var(--s-1) var(--s-1) var(--s-3);
  }
  .pick {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-weight: 500;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The whole tile is the select button's hit area. */
  .pick::before {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: var(--r-panel);
  }
  .pending {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: var(--s-1);
    height: 20px;
    padding: 0 var(--s-2);
    border-radius: var(--r-full);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .count {
    font-variant-numeric: tabular-nums;
    flex: none;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .clear {
    position: relative;
    z-index: 1;
    display: inline-flex;
  }
</style>
