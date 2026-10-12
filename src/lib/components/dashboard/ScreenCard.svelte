<!--
  One screen on the dashboard: a live thumbnail of the REAL /screen/<id> URL (scaled down from the
  screen's native size over the transparency checkerboard), a tally dot + name, the mono URL and a
  copy button. A selectable object: viewfinder brackets on keyboard focus (of its link) and while
  `selected`. The whole card is the link; the copy button sits above it.

  The grid (ScreensGrid) decides whether the iframe exists (`live`); this component never creates
  one on its own.

  Props:
    view: LiveScreenView
    url: string | null       local screen URL (null while the server isn't running)
    lanUrl: string | null    LAN screen URL when LAN access is on; shown and copied instead
    tally: ScreenTally       program | preview | idle
    live: boolean            mount the preview iframe
    note: string | null      muted line over the checkerboard when not live ("Preview paused")
    width: number            thumbnail box width in px (for the scale)
    selected?: boolean
    track: Attachment        observer registration for the thumbnail box (from the grid)
-->
<script lang="ts">
  import type { Attachment } from "svelte/attachments";
  import Copy from "lucide-svelte/icons/copy";
  import IconButton from "$lib/components/IconButton.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { copyUrl } from "$lib/stores/server.svelte";
  import type { LiveScreenView } from "$lib/types";
  import { TALLY_LABELS, type ScreenTally } from "./data";

  type Props = {
    view: LiveScreenView;
    url: string | null;
    lanUrl: string | null;
    tally: ScreenTally;
    live: boolean;
    note: string | null;
    width: number;
    selected?: boolean;
    track: Attachment<HTMLElement>;
  };
  let { view, url, lanUrl, tally, live, note, width, selected = false, track }: Props = $props();

  const uid = $props.id();
  const info = $derived(view.screen);
  const nativeW = $derived(info.width || 1920);
  const nativeH = $derived(info.height || 1080);
  const scale = $derived(width > 0 ? width / nativeW : 0);
  const shown = $derived(lanUrl ?? url);
</script>

<article
  class="card vf vf-live"
  data-selected={selected || undefined}
  data-tally={tally === "program" ? "program" : undefined}
  aria-labelledby="{uid}-name"
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
      <div class="paused">
        <span class="paused-name">{info.name}</span>
        <span class="paused-note">{note}</span>
      </div>
    {/if}
  </div>

  <div class="foot">
    <div class="title">
      <TallyDot state={tally} label={TALLY_LABELS[tally]} />
      <a id="{uid}-name" class="open" href="/screens/{info.id}">{info.name}</a>
      <span class="res mono">{nativeW}×{nativeH}</span>
    </div>
    <div class="url-row">
      <span class="url mono" title={shown ?? undefined}>{shown ?? `/screen/${info.id}`}</span>
      <IconButton
        label="Copy URL of {info.name}"
        size="sm"
        tooltip="left"
        disabled={!shown}
        onclick={() => shown && copyUrl(shown)}
      >
        <Copy size={14} />
      </IconButton>
    </div>
  </div>
</article>

<style>
  .card {
    --vf-radius: var(--r-panel);
    display: flex;
    flex-direction: column;
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
  /* Keyboard focus on the card's link draws the card's focus ring (the link itself gets none). */
  .card:has(.open:focus-visible)::after {
    opacity: 1;
  }
  .open:focus-visible {
    outline: none;
  }

  .thumb {
    position: relative;
    overflow: hidden;
    border-radius: var(--r-panel) var(--r-panel) 0 0;
    border-bottom: 1px solid var(--line);
  }
  .thumb iframe {
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
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    padding: var(--s-3);
    text-align: center;
  }
  .paused-name,
  .paused-note {
    max-width: 100%;
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .paused-name {
    font-weight: 500;
  }
  .paused-note {
    color: var(--text-muted);
    font-size: var(--fs-sm);
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
    height: var(--control-h-sm);
    padding-right: var(--s-1);
  }
  .open {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-weight: 500;
    text-decoration: none;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The whole card is the link's hit area. */
  .open::before {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: var(--r-panel);
  }
  .res {
    flex: none;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .url-row {
    /* Above the link's hit area, so the copy button and URL selection work. */
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
  }
  .url {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
    user-select: all;
  }
</style>
