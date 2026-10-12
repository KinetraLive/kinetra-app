<!--
  Canvas: the screen editor's centre. ONE LivePreview of the real `/screen/<id>` (the only iframe
  in the editor), fitted into the available area, with an overlay of layer boxes in screen
  coordinates scaled onto the same stage. Everything drawn over the preview is editor-only.

  - Click a box to select it; Shift/Ctrl+click adds or removes it; drag on empty canvas to select
    the boxes a marquee touches (Shift/Ctrl adds). A single selected box gets viewfinder brackets
    and 8 resize handles; a multi-selection shows its bounds.
  - Drag a box to move the selection, a handle to resize. Resizing scales: a layer that isn't
    explicitly Reflow keeps its graphic's aspect (the design box's, else its own box's; Shift
    stretches freely) and becomes Scale to fit (`onresizestart` on the first frame of the drag); a
    Reflow layer resizes freely (Shift keeps the ratio). The layer's aspect lock (`lockedIds`, the
    effective lock) replaces the default. Alt resizes from the centre, and no side goes below
    LAYER_SIZE_MIN (16 screen px; a smaller layer keeps its own size as the floor). Boxes snap (when `view.snap`; 6 CSS px at any
    zoom; Alt turns it off for a move, Ctrl for a resize) by
    their edges and centre to the screen edges and centre, action / title safe, other layers'
    edges and centres, ruler guides, visible custom safe areas and crop, the grid, and to equal
    spacing in their row / column. Thin accent lines show the alignment; smart guides show the
    distances to the neighbours (mono screen px) and equal-spacing markers. A modifier pressed or
    released mid-drag applies at once. A mono readout next to the dragged box (or the selection's
    bounds) shows `X 120 Y 840` while moving, `W 640 H 180` while resizing; it hides on drop.
  - Hit-testing is by box, not by pixel: a layer's whole rect takes the pointer, in z-order, so a
    layer in front covers the ones behind it even where its template draws nothing. A selected box
    is raised above the others, so a layer picked in the Layers panel (or by a marquee) can be
    dragged even when it sits behind another.
  - Arrow keys nudge the selection 1 px (Shift: 10 px); Delete removes it; Escape deselects. A
    marquee focuses the selection, so arrows work right after it.

  Rulers, guides, the view overlays and smart guides come from a lazy chunk (overlays.ts); until it
  arrives, plain snapping works and nothing else draws.

  During a drag only the dragged boxes change (transforms; size too while resizing); pointer moves
  are coalesced to one update per animation frame with no layout reads. `ondrag` reports every
  frame's rects (the editor throttles what goes to the output), `ondrop` the final ones. No timers
  or frames at rest.

  Props:
    screen: { width, height }
    layers: ScreenLayer[]           doc order (bottom first)
    selectedIds: string[]
    src: string | null              screen URL (null while the server isn't running)
    status: string                  shown over the checkerboard when there is no iframe
    view: ScreenEditorView          snapping, guides, grid, safe areas, crop
    guides: ScreenGuides
    safeAreas: SafeArea[]
    lockedIds: ReadonlySet<string>  layers whose aspect ratio is locked (the effective lock)
    designOf: (templateId: string) => { w, h } | null   a template's design box (null: none / unknown)
    onresizestart: (id: string) => void   a resize drag of the layer began (the editor makes it fit)
    onselect: (ids: string[]) => void
    ondrag: (changes: LayerRect[]) => void
    ondrop: (changes: LayerRect[]) => void   end of a drag, a nudge
    ondelete: (ids: string[]) => void
    onguides: (next: ScreenGuides) => void
-->
<script module lang="ts">
  import type { Rect } from "$lib/types";

  export interface LayerRect {
    id: string;
    rect: Rect;
  }

  // Rulers, guides, overlays and smart guides: requested when this module loads.
  const overlaysChunk = import("./overlays");
</script>

<script lang="ts">
  import LivePreview from "$lib/components/LivePreview.svelte";
  import type { SafeArea, ScreenEditorView, ScreenGuides, ScreenLayer } from "$lib/types";
  import {
    FrameQueue,
    HANDLES,
    boundsOf,
    clampRect,
    cropRect,
    fitStage,
    moveGroup,
    moveRect,
    nudge,
    dragKeepsAspect,
    graphicRatio,
    readoutPlace,
    readoutText,
    resizeSnap,
    scalesOnResize,
    sameRect,
    snapMove,
    snapTargets,
    snapThreshold,
    toScreen,
    type Guide,
    type Handle,
    type ReadoutPlace,
    type SnapTargets,
  } from "./geometry";
  import { tick } from "svelte";
  import { LAYER_SIZE_MIN } from "./limits";
  import type { Smart, SmartContext } from "./smart";

  let {
    screen,
    layers,
    selectedIds,
    src,
    status,
    view,
    guides: rulerGuides,
    safeAreas,
    lockedIds,
    designOf,
    onresizestart,
    onselect,
    ondrag,
    ondrop,
    ondelete,
    onguides,
  }: {
    screen: { width: number; height: number };
    layers: readonly ScreenLayer[];
    selectedIds: readonly string[];
    src: string | null;
    status: string;
    view: ScreenEditorView;
    guides: ScreenGuides;
    safeAreas: readonly SafeArea[];
    lockedIds: ReadonlySet<string>;
    designOf: (templateId: string) => { w: number; h: number } | null | undefined;
    onresizestart: (id: string) => void;
    onselect: (ids: string[]) => void;
    ondrag: (changes: LayerRect[]) => void;
    ondrop: (changes: LayerRect[]) => void;
    ondelete: (ids: string[]) => void;
    onguides: (next: ScreenGuides) => void;
  } = $props();

  let O = $state.raw<typeof import("./overlays") | null>(null);
  void overlaysChunk.then((m) => (O = m));

  const size = $derived({ w: screen.width, h: screen.height });

  // ---- Fit ----------------------------------------------------------------------------------------

  let area: HTMLDivElement | undefined = $state();
  let availW = $state(0);
  let availH = $state(0);
  const stage = $derived(fitStage(availW, availH, screen.width, screen.height));
  const scale = $derived(stage.scale);

  $effect(() => {
    if (!area) return;
    const ro = new ResizeObserver((entries) => {
      const last = entries[entries.length - 1];
      if (!last) return;
      availW = last.contentRect.width;
      availH = last.contentRect.height;
    });
    ro.observe(area);
    return () => ro.disconnect();
  });

  // ---- Snap targets beyond the screen and layers ----------------------------------------------------

  function extraTargets() {
    const crop = cropRect(size, view.crop);
    return {
      x: view.guides ? rulerGuides.x : [],
      y: view.guides ? rulerGuides.y : [],
      rects: [...(view.customSafe ? safeAreas.map((a) => a.rect) : []), ...(crop ? [crop] : [])],
      grid: view.grid ? view.gridSize : undefined,
    };
  }

  // ---- Drag -----------------------------------------------------------------------------------------

  interface Drag {
    mode: "move" | Handle;
    ids: string[];
    /** Each dragged layer's rect at the start (same order as ids). */
    starts: Rect[];
    /** The selection's bounds (move) or the resized rect. */
    start: Rect;
    px: number;
    py: number;
    targets: SnapTargets;
    ctx: SmartContext | null;
    lock: boolean;
    /** Resizing scales the graphic (the layer is or becomes Scale to fit). */
    scales: boolean;
    /** The ratio a resize keeps (the graphic's for a scaling layer, else the box's). */
    ratio: number | undefined;
    moved: boolean;
    /** A plain click on a box of a multi-selection selects just it (if it wasn't a drag). */
    collapse: string | null;
  }
  interface PointerSample {
    x: number;
    y: number;
    shift: boolean;
    alt: boolean;
    ctrl: boolean;
  }

  let overlay: HTMLDivElement | undefined = $state();
  let drag: Drag | null = null;
  let dragRects = $state.raw<Map<string, Rect> | null>(null);
  let snapLines = $state.raw<Guide[]>([]);
  let smart = $state.raw<Smart | null>(null);
  /** Live position (move) or size (resize) next to the dragged box / selection, set per frame. */
  let readout = $state.raw<{ text: string; at: ReadoutPlace } | null>(null);
  /** Focus moves to a box during a press; that must not reselect. */
  let pressing = false;
  /** The drag's latest pointer sample, replayed when a modifier changes without a move. */
  let lastSample: PointerSample | null = null;

  /** Pixels the pointer must travel before a press becomes a drag (a plain click only selects). */
  const DRAG_SLOP_PX = 2;

  const frames = new FrameQueue<PointerSample>((p) => {
    const d = drag;
    if (!d) return;
    lastSample = p;
    if (!d.moved && Math.abs(p.x - d.px) < DRAG_SLOP_PX && Math.abs(p.y - d.py) < DRAG_SLOP_PX) return;
    if (!d.moved && d.mode !== "move") onresizestart(d.ids[0]);
    d.moved = true;
    const dx = toScreen(p.x - d.px, scale);
    const dy = toScreen(p.y - d.py, scale);
    let rects: Rect[];
    if (d.mode === "move") {
      const threshold = p.alt || !view.snap ? 0 : snapThreshold(scale);
      const raw = moveRect(d.start, dx, dy);
      let s = snapMove(raw, d.targets, threshold);
      if (d.ctx && O) s = O.snapSpacing(raw, s, d.ctx, threshold);
      rects = moveGroup(d.starts, s.rect.x - d.start.x, s.rect.y - d.start.y, size);
      snapLines = s.guides;
    } else {
      const threshold = p.ctrl || !view.snap ? 0 : snapThreshold(scale);
      const opts = { keepAspect: dragKeepsAspect(d.scales, d.lock, p.shift), fromCenter: p.alt, min: LAYER_SIZE_MIN, ratio: d.ratio };
      const s = resizeSnap(d.start, d.mode, dx, dy, opts, d.targets, threshold);
      rects = [clampRect(s.rect, size)];
      snapLines = s.guides;
    }
    const bounds = boundsOf(rects);
    smart = d.ctx && O && bounds ? O.smartGuides(bounds, d.ctx) : null;
    readout = bounds
      ? { text: readoutText(d.mode === "move" ? "move" : "size", bounds), at: readoutPlace(bounds, size, scale) }
      : null;
    dragRects = new Map(d.ids.map((id, i) => [id, rects[i]]));
    ondrag(d.ids.map((id, i) => ({ id, rect: rects[i] })));
  });

  function capture(e: PointerEvent) {
    try {
      overlay?.setPointerCapture?.(e.pointerId);
    } catch {
      // Synthetic pointers can't be captured; the overlay still receives the moves.
    }
  }

  function begin(e: PointerEvent, layer: ScreenLayer, mode: "move" | Handle) {
    if (e.button !== 0 || scale <= 0) return;
    e.preventDefault();
    e.stopPropagation();
    const additive = e.shiftKey || e.ctrlKey || e.metaKey;
    const was = selectedIds.includes(layer.id);
    let ids: string[];
    let collapse: string | null = null;
    if (mode !== "move") {
      ids = [layer.id];
    } else if (additive) {
      ids = was ? selectedIds.filter((x) => x !== layer.id) : [...selectedIds, layer.id];
      onselect(ids);
      if (was) return; // removed from the selection: nothing to drag
    } else if (was) {
      ids = [...selectedIds];
      if (ids.length > 1) collapse = layer.id;
    } else {
      ids = [layer.id];
      onselect(ids);
    }
    pressing = true;
    (e.currentTarget as HTMLElement).closest<HTMLElement>(".box")?.focus({ preventScroll: true });
    pressing = false;
    capture(e);
    const set = new Set(ids);
    const dragged = layers.filter((l) => set.has(l.id));
    const others = layers.filter((l) => !set.has(l.id)).map((l) => l.rect);
    const starts = dragged.map((l) => l.rect);
    drag = {
      mode,
      ids: dragged.map((l) => l.id),
      starts,
      start: mode === "move" ? (boundsOf(starts) ?? layer.rect) : layer.rect,
      px: e.clientX,
      py: e.clientY,
      targets: snapTargets(size, others, extraTargets()),
      ctx: O ? O.smartContext(size, others) : null,
      lock: mode !== "move" && lockedIds.has(layer.id),
      // Media layers resize freely: their fit (fill / fit / stretch) places the picture.
      scales: layer.kind !== "media" && scalesOnResize(layer.sizing),
      ratio:
        mode !== "move" && layer.kind !== "media" && scalesOnResize(layer.sizing)
          ? graphicRatio(layer.rect, designOf(layer.templateId))
          : undefined,
      moved: false,
      collapse,
    };
    lastSample = null;
    window.addEventListener("keydown", onDragKey, true);
    window.addEventListener("keyup", onDragKey, true);
  }

  /**
   * Shift / Alt / Ctrl pressed or released mid-drag apply at once (no pointer move needed): the
   * last sample runs again with the new modifiers. Listening only while a drag runs; Alt's default
   * (the window menu on release) is suppressed so it can't steal focus from the drag.
   */
  function onDragKey(e: KeyboardEvent) {
    if (e.key !== "Alt" && e.key !== "Shift" && e.key !== "Control" && e.key !== "Meta") return;
    if (e.key === "Alt") e.preventDefault();
    const p = lastSample;
    if (drag && p) frames.push({ ...p, shift: e.shiftKey, alt: e.altKey, ctrl: e.ctrlKey || e.metaKey });
  }

  function stopDragKeys() {
    window.removeEventListener("keydown", onDragKey, true);
    window.removeEventListener("keyup", onDragKey, true);
    lastSample = null;
  }

  function onpointermove(e: PointerEvent) {
    if (drag) frames.push({ x: e.clientX, y: e.clientY, shift: e.shiftKey, alt: e.altKey, ctrl: e.ctrlKey || e.metaKey });
    else if (marquee) marqueeFrames.push({ x: e.clientX, y: e.clientY, shift: false, alt: false, ctrl: false });
  }

  function finish() {
    if (marquee) {
      endMarquee();
      return;
    }
    if (!drag) return;
    frames.flush();
    stopDragKeys();
    const d = drag;
    const rects = dragRects;
    drag = null;
    dragRects = null;
    snapLines = [];
    smart = null;
    readout = null;
    if (d.moved && rects) {
      const changes = d.ids
        .map((id, i) => ({ id, rect: rects.get(id) ?? d.starts[i] }))
        .filter((c, i) => !sameRect(c.rect, d.starts[i]));
      if (changes.length > 0) ondrop(changes);
    } else if (!d.moved && d.collapse) {
      onselect([d.collapse]);
    }
  }

  $effect(() => () => {
    frames.cancel();
    marqueeFrames.cancel();
    stopDragKeys();
  });

  // ---- Marquee ------------------------------------------------------------------------------------

  interface Marquee {
    origin: DOMRect;
    x0: number;
    y0: number;
    base: string[];
  }
  let marquee: Marquee | null = null;
  /** The marquee in screen px while it is drawn. */
  let band = $state.raw<Rect | null>(null);

  const marqueeFrames = new FrameQueue<PointerSample>((p) => {
    const m = marquee;
    if (!m) return;
    if (!band && Math.abs(p.x - m.x0) < DRAG_SLOP_PX && Math.abs(p.y - m.y0) < DRAG_SLOP_PX) return;
    const ax = toScreen(m.x0 - m.origin.left, scale);
    const ay = toScreen(m.y0 - m.origin.top, scale);
    const bx = toScreen(p.x - m.origin.left, scale);
    const by = toScreen(p.y - m.origin.top, scale);
    band = { x: Math.min(ax, bx), y: Math.min(ay, by), w: Math.abs(bx - ax), h: Math.abs(by - ay) };
  });

  function onOverlayDown(e: PointerEvent) {
    if (e.button !== 0 || e.target !== overlay || !overlay || scale <= 0) return;
    const additive = e.shiftKey || e.ctrlKey || e.metaKey;
    capture(e);
    marquee = { origin: overlay.getBoundingClientRect(), x0: e.clientX, y0: e.clientY, base: additive ? [...selectedIds] : [] };
  }

  function endMarquee() {
    marqueeFrames.flush();
    const m = marquee;
    const b = band;
    marquee = null;
    band = null;
    if (!m) return;
    if (!b) {
      // A plain press on empty canvas clears the selection (Shift/Ctrl keeps it).
      onselect(m.base);
      return;
    }
    const hit = layers
      .filter((l) => {
        const r = l.rect;
        return r.x < b.x + b.w && b.x < r.x + r.w && r.y < b.y + b.h && b.y < r.y + r.h;
      })
      .map((l) => l.id);
    const next = [...m.base, ...hit.filter((id) => !m.base.includes(id))];
    onselect(next);
    // Focus the selection's tab stop so arrow keys nudge it right away (no layout read).
    if (next.length > 0) void tick().then(() => focusBox(next[0]));
  }

  function focusBox(id: string) {
    const el = [...(overlay?.querySelectorAll<HTMLElement>(".box") ?? [])].find((b) => b.dataset.layer === id);
    pressing = true;
    el?.focus({ preventScroll: true });
    pressing = false;
  }

  // ---- Keyboard -------------------------------------------------------------------------------------

  function onBoxKey(e: KeyboardEvent, layer: ScreenLayer) {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const ids = selectedIds.includes(layer.id) ? selectedIds : [layer.id];
    const step = nudge({ x: 0, y: 0, w: 1, h: 1 }, e.key, e.shiftKey);
    if (step) {
      e.preventDefault();
      const dragged = layers.filter((l) => ids.includes(l.id));
      const moved = moveGroup(
        dragged.map((l) => l.rect),
        step.x,
        step.y,
        size,
      );
      const changes = dragged
        .map((l, i) => ({ id: l.id, rect: moved[i] }))
        .filter((c, i) => !sameRect(c.rect, dragged[i].rect));
      if (changes.length > 0) ondrop(changes);
    } else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      ondelete([...ids]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      onselect([]);
      (e.currentTarget as HTMLElement).blur();
    }
  }

  /** Roving tab stop: the first selected box, else the front-most one. */
  const tabStop = $derived(selectedIds.find((id) => layers.some((l) => l.id === id)) ?? layers.at(-1)?.id ?? null);
  const single = $derived(selectedIds.length === 1 ? selectedIds[0] : null);
  const selectionBounds = $derived(
    selectedIds.length > 1
      ? boundsOf(layers.filter((l) => selectedIds.includes(l.id)).map((l) => dragRects?.get(l.id) ?? l.rect))
      : null,
  );

  const CURSORS: Record<Handle, string> = {
    nw: "nwse-resize",
    se: "nwse-resize",
    ne: "nesw-resize",
    sw: "nesw-resize",
    n: "ns-resize",
    s: "ns-resize",
    e: "ew-resize",
    w: "ew-resize",
  };
</script>

<div class="area" bind:this={area}>
  <div class="stage" style:width="{stage.w}px" style:height="{stage.h}px">
    <LivePreview {src} title="Screen output" width={screen.width} height={screen.height}>
      <span>{status}</span>
    </LivePreview>

    {#if O && scale > 0}
      <O.ViewOverlays screen={size} {scale} {view} {safeAreas} />
    {/if}

    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="overlay"
      bind:this={overlay}
      onpointerdown={onOverlayDown}
      {onpointermove}
      onpointerup={finish}
      onpointercancel={finish}
      onlostpointercapture={finish}
    >
      {#each layers as layer (layer.id)}
        {@const r = dragRects?.get(layer.id) ?? layer.rect}
        {@const selected = selectedIds.includes(layer.id)}
        <div
          class="box vf vf-live"
          class:selected
          class:dragging={dragRects?.has(layer.id)}
          data-selected={selected ? "" : undefined}
          data-layer={layer.id}
          role="button"
          tabindex={layer.id === tabStop ? 0 : -1}
          aria-pressed={selected}
          aria-label="{layer.name}, {r.w} by {r.h} at {r.x}, {r.y}"
          style:transform="translate({r.x * scale}px, {r.y * scale}px)"
          style:width="{r.w * scale}px"
          style:height="{r.h * scale}px"
          onpointerdown={(e) => begin(e, layer, "move")}
          onkeydown={(e) => onBoxKey(e, layer)}
          onfocus={() => {
            if (!pressing && !selectedIds.includes(layer.id)) onselect([layer.id]);
          }}
        >
          {#if layer.id === single}
            <span class="tag" aria-hidden="true">{layer.name}</span>
            {#each HANDLES as h (h)}
              <span
                class="handle {h}"
                data-handle={h}
                style:cursor={CURSORS[h]}
                aria-hidden="true"
                onpointerdown={(e) => begin(e, layer, h)}
              ></span>
            {/each}
          {/if}
        </div>
      {/each}

      {#if selectionBounds}
        <div
          class="bounds"
          aria-hidden="true"
          style:transform="translate({selectionBounds.x * scale}px, {selectionBounds.y * scale}px)"
          style:width="{selectionBounds.w * scale}px"
          style:height="{selectionBounds.h * scale}px"
        >
          <span class="tag">{selectedIds.length} layers</span>
        </div>
      {/if}

      {#if band}
        <div
          class="marquee"
          aria-hidden="true"
          style:transform="translate({band.x * scale}px, {band.y * scale}px)"
          style:width="{band.w * scale}px"
          style:height="{band.h * scale}px"
        ></div>
      {/if}

      {#if readout}
        <div
          class="readout mono"
          data-readout
          aria-hidden="true"
          style:transform="translate({readout.at.x}px, {readout.at.y}px) translate({readout.at.tx}%, {readout.at.ty}%)"
        >{readout.text}</div>
      {/if}

      {#each snapLines as g, i (i)}
        <div
          class="guide {g.axis}"
          aria-hidden="true"
          style:transform={g.axis === "x" ? `translateX(${g.pos * scale}px)` : `translateY(${g.pos * scale}px)`}
        ></div>
      {/each}
    </div>

    {#if O && scale > 0}
      {#if view.guides}
        <O.Rulers screen={size} {scale} guides={rulerGuides} {onguides} />
      {/if}
      <O.SmartGuides {smart} {scale} />
    {/if}
  </div>
</div>

<style>
  .area {
    display: grid;
    place-items: center;
    min-width: 0;
    min-height: 0;
    padding: var(--s-5);
    overflow: hidden;
    background: var(--bg);
  }
  .stage {
    position: relative;
  }
  .overlay {
    position: absolute;
    inset: 0;
    touch-action: none;
    user-select: none;
  }

  .box {
    --vf-radius: 0px;
    position: absolute;
    top: 0;
    left: 0;
    box-shadow: inset 0 0 0 1px var(--line-strong);
    cursor: move;
    contain: layout style;
  }
  .box:hover {
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .box.selected {
    z-index: 1;
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .box.dragging {
    will-change: transform;
  }

  .bounds,
  .marquee {
    position: absolute;
    top: 0;
    left: 0;
    z-index: 1;
    pointer-events: none;
  }
  .bounds {
    outline: 1px dashed var(--accent);
  }
  .marquee {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent);
  }

  .tag {
    position: absolute;
    top: 0;
    left: 0;
    max-width: 100%;
    overflow: hidden;
    padding: 0 var(--s-1);
    background: var(--accent);
    color: var(--accent-ink);
    font-size: var(--fs-xs);
    line-height: 16px;
    text-overflow: ellipsis;
    white-space: nowrap;
    pointer-events: none;
  }
  .bounds .tag {
    top: auto;
    bottom: 100%;
  }

  .handle {
    position: absolute;
    width: 9px;
    height: 9px;
    border-radius: 2px;
    background: var(--accent);
    box-shadow: inset 0 0 0 1px var(--accent-ink);
    transform: translate(-50%, -50%);
  }
  .handle.nw {
    left: 0;
    top: 0;
  }
  .handle.n {
    left: 50%;
    top: 0;
  }
  .handle.ne {
    left: 100%;
    top: 0;
  }
  .handle.e {
    left: 100%;
    top: 50%;
  }
  .handle.se {
    left: 100%;
    top: 100%;
  }
  .handle.s {
    left: 50%;
    top: 100%;
  }
  .handle.sw {
    left: 0;
    top: 100%;
  }
  .handle.w {
    left: 0;
    top: 50%;
  }

  /* Above the smart guides' labels (z-index 3), never taking the pointer. */
  .readout {
    position: absolute;
    top: 0;
    left: 0;
    z-index: 4;
    padding: 0 var(--s-1);
    border-radius: 3px;
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-size: var(--fs-xs);
    line-height: 18px;
    white-space: nowrap;
    pointer-events: none;
  }

  .guide {
    position: absolute;
    top: 0;
    left: 0;
    z-index: 2;
    background: var(--accent);
    pointer-events: none;
  }
  .guide.x {
    width: 1px;
    height: 100%;
  }
  .guide.y {
    width: 100%;
    height: 1px;
  }
</style>
