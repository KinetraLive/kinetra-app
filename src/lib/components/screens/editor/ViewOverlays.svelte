<!--
  ViewOverlays: the layout editor's static overlays over the canvas (part of the lazy overlays
  chunk): the grid, action safe (93 %) and title safe (90 %), the centre cross, the screen's custom
  safe areas and an aspect crop (centred, dimmed outside). One SVG in CSS px; the grid is an SVG
  pattern (no element per line). Re-renders only when the view, the zoom or the areas change, never
  per drag frame. Editor-only: it sits over the preview iframe, nothing reaches the output.

  Props:
    screen: { w, h }          screen px
    scale: number             CSS px per screen px
    view: ScreenEditorView
    safeAreas: SafeArea[]
-->
<script lang="ts">
  import type { SafeArea, ScreenEditorView } from "$lib/types";
  import { ACTION_SAFE, TITLE_SAFE, cropRect, safeRect, type Size } from "./shared";
  import { visibleGrid } from "./ruler";

  let {
    screen,
    scale,
    view,
    safeAreas,
  }: { screen: Size; scale: number; view: ScreenEditorView; safeAreas: readonly SafeArea[] } = $props();

  const uid = $props.id();
  const W = $derived(screen.w * scale);
  const H = $derived(screen.h * scale);
  const grid = $derived(view.grid ? visibleGrid(view.gridSize, scale) * scale : 0);
  const px = (v: number) => Math.round(v * scale) + 0.5;
  const box = (r: { x: number; y: number; w: number; h: number }) => ({
    x: px(r.x),
    y: px(r.y),
    w: Math.max(0, Math.round(r.w * scale) - 1),
    h: Math.max(0, Math.round(r.h * scale) - 1),
  });
  const action = $derived(box(safeRect(screen, ACTION_SAFE)));
  const title = $derived(box(safeRect(screen, TITLE_SAFE)));
  const crop = $derived(cropRect(screen, view.crop));
  const cropBox = $derived(crop ? box(crop) : null);
  const arm = $derived(Math.max(8, Math.min(W, H) * 0.04));
</script>

<svg class="overlays" width={W} height={H} aria-hidden="true">
  {#if grid > 0}
    <defs>
      <pattern id="{uid}-grid" width={grid} height={grid} patternUnits="userSpaceOnUse">
        <path class="grid" d="M{grid - 0.5} 0V{grid}M0 {grid - 0.5}H{grid}" />
      </pattern>
    </defs>
    <rect data-overlay="grid" width={W} height={H} fill="url(#{uid}-grid)" />
  {/if}

  {#if cropBox}
    <path
      data-overlay="crop"
      class="dim"
      fill-rule="evenodd"
      d="M0 0H{W}V{H}H0Z M{cropBox.x} {cropBox.y}h{cropBox.w}v{cropBox.h}h{-cropBox.w}Z"
    />
    <rect class="crop" x={cropBox.x} y={cropBox.y} width={cropBox.w} height={cropBox.h} />
    <text class="label mono" text-anchor="end" x={cropBox.x + cropBox.w - 4} y={cropBox.y + 14}>{view.crop}</text>
  {/if}

  {#if view.actionSafe}
    <rect data-overlay="action-safe" class="safe" x={action.x} y={action.y} width={action.w} height={action.h} />
    <text class="label" x={action.x + 4} y={action.y + action.h - 5}>Action safe 93%</text>
  {/if}
  {#if view.titleSafe}
    <rect data-overlay="title-safe" class="safe" x={title.x} y={title.y} width={title.w} height={title.h} />
    <text class="label" x={title.x + 4} y={title.y + 14}>Title safe 90%</text>
  {/if}

  {#if view.centerCross}
    <path data-overlay="center-cross" class="cross" d="M{px(screen.w / 2) - arm} {px(screen.h / 2)}h{arm * 2}M{px(screen.w / 2)} {px(screen.h / 2) - arm}v{arm * 2}" />
  {/if}

  {#if view.customSafe}
    {#each safeAreas as a (a.id)}
      {@const b = box(a.rect)}
      <rect data-overlay="safe-area" class="custom" x={b.x} y={b.y} width={b.w} height={b.h} />
      <text class="label" x={b.x + 4} y={b.y + 14}>{a.name}</text>
    {/each}
  {/if}
</svg>

<style>
  .overlays {
    position: absolute;
    top: 0;
    left: 0;
    overflow: visible;
    pointer-events: none;
  }
  .grid {
    fill: none;
    stroke: var(--text-muted);
    stroke-width: 1;
    opacity: 0.45;
  }
  .dim {
    fill: var(--overlay);
  }
  .crop {
    fill: none;
    stroke: var(--text);
    stroke-width: 1;
  }
  .safe {
    fill: none;
    stroke: var(--text);
    stroke-width: 1;
    stroke-dasharray: 6 4;
    opacity: 0.75;
  }
  .custom {
    fill: none;
    stroke: var(--text);
    stroke-width: 1;
    stroke-dasharray: 2 3;
  }
  .cross {
    fill: none;
    stroke: var(--text);
    stroke-width: 1;
  }
  .label {
    fill: var(--text);
    font-family: var(--font-body);
    font-size: var(--fs-xs);
    paint-order: stroke;
    stroke: var(--bg);
    stroke-width: 3px;
    stroke-linejoin: round;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
