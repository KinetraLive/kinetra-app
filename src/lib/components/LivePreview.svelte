<!--
  LivePreview: a screen rendered by the REAL renderer (`/screen/<id>` in an iframe), scaled from its
  native size to the box width, over the transparency checkerboard. One rendering path for OBS and
  the app.

  Performance (hard rules): an iframe is a whole WebView2 renderer process
  (~25 MB). It exists only while the box is in the viewport AND the window is visible AND `src` is
  set (server running); otherwise it is removed, never hidden. One IntersectionObserver and one
  ResizeObserver while mounted; nothing runs at rest.

  Props:
    src: string | null        the screen URL (null = server not running: checkerboard only)
    title: string             accessible name of the iframe, e.g. "Template preview"
    width?, height?: number   the screen's native size (default 1920×1080)
    selected?: boolean        viewfinder brackets (e.g. while on air / selected)
    children?: Snippet        shown over the checkerboard when there is no iframe (status text)
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    src,
    title,
    width = 1920,
    height = 1080,
    selected = false,
    children,
  }: {
    src: string | null;
    title: string;
    width?: number;
    height?: number;
    selected?: boolean;
    children?: Snippet;
  } = $props();

  let box: HTMLDivElement | undefined = $state();
  let boxW = $state(0);
  let intersecting = $state(false);
  let pageVisible = $state(typeof document === "undefined" || document.visibilityState === "visible");

  const scale = $derived(boxW > 0 ? boxW / width : 0);
  /** The iframe exists only under these conditions; otherwise it is removed, not hidden. */
  const live = $derived(intersecting && pageVisible && src !== null);

  $effect(() => {
    if (!box) return;
    const io = new IntersectionObserver((entries) => {
      const last = entries[entries.length - 1];
      if (last) intersecting = last.isIntersecting;
    });
    const ro = new ResizeObserver((entries) => {
      const last = entries[entries.length - 1];
      if (last) boxW = last.contentRect.width;
    });
    io.observe(box);
    ro.observe(box);
    return () => {
      io.disconnect();
      ro.disconnect();
    };
  });

  function onvisibilitychange() {
    pageVisible = document.visibilityState === "visible";
  }
</script>

<svelte:document {onvisibilitychange} />

<div
  class="preview checker vf vf-live"
  data-selected={selected ? "" : undefined}
  bind:this={box}
  style:aspect-ratio="{width} / {height}"
>
  {#if live && src}
    <!-- App previews are muted and don't count toward media readiness (`?mute=1`). -->
    <iframe
      src="{src}?mute=1"
      {title}
      tabindex="-1"
      sandbox="allow-scripts allow-same-origin"
      style:width="{width}px"
      style:height="{height}px"
      style:transform="scale({scale})"
    ></iframe>
  {:else if children}
    <div class="status">{@render children()}</div>
  {/if}
</div>

<style>
  .preview {
    --vf-radius: var(--r-panel);
    position: relative;
    width: 100%;
    overflow: visible;
    border-radius: var(--r-panel);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  iframe {
    position: absolute;
    top: 0;
    left: 0;
    border: 0;
    background: transparent;
    transform-origin: 0 0;
    pointer-events: none;
    /* The renderer page uses the default (light) colour scheme. When an iframe's scheme differs
       from its embedder's, the browser paints it with an opaque canvas, so in the dark app theme
       the transparent output would show as solid white. Match the renderer's scheme. */
    color-scheme: light;
  }
  .status {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: var(--s-4);
    color: var(--text-muted);
    font-size: var(--fs-sm);
    text-align: center;
  }
</style>
