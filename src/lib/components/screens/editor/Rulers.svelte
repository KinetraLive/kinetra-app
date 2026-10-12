<!--
  Rulers: the layout editor's rulers and ruler guides (part of the lazy overlays chunk). Rulers run
  along the top and left edges of the canvas in screen px (mono labels; ticks follow the zoom; one
  SVG path each, re-rendered on resize / zoom only).

  - Drag from the top ruler for a horizontal guide, from the left ruler for a vertical one.
  - Drag a guide to move it; drop it back onto its ruler to delete it.
  - Double-click a guide (or Enter on it) to type its exact position (mono input; Enter applies,
    Esc cancels). Focused guide: arrow keys move it 1 px (Shift: 10 px), Delete removes it.
  - Enter or Space on a ruler adds a guide at the centre and opens its input.

  Positions are whole screen px within -size..2×size. Every change calls `onguides` with the full,
  sorted set (the editor saves it in the screen file). Editor-only: nothing reaches the output.

  Props:
    screen: { w, h }
    scale: number                  CSS px per screen px
    guides: ScreenGuides
    onguides: (next: ScreenGuides) => void
-->
<script lang="ts">
  import { tick } from "svelte";
  import type { ScreenGuides } from "$lib/types";
  import { FrameQueue, SCREEN_GUIDES_MAX, type Size } from "./shared";
  import { rulerTicks } from "./ruler";

  let {
    screen,
    scale,
    guides,
    onguides,
  }: { screen: Size; scale: number; guides: ScreenGuides; onguides: (next: ScreenGuides) => void } = $props();

  type Axis = "x" | "y";
  const AXES: readonly Axis[] = ["x", "y"];
  /** Ruler thickness, CSS px. */
  const T = 20;

  let root: HTMLDivElement | undefined = $state();
  let editInput: HTMLInputElement | undefined = $state();

  const top = $derived(rulerTicks(screen.w, scale, T));
  const left = $derived(rulerTicks(screen.h, scale, T, true));
  const sizeOf = (axis: Axis) => (axis === "x" ? screen.w : screen.h);
  const clampPos = (axis: Axis, v: number) => Math.min(2 * sizeOf(axis), Math.max(-sizeOf(axis), Math.round(v)));
  const label = (axis: Axis) => (axis === "x" ? "Vertical" : "Horizontal");

  function set(axis: Axis, list: number[]) {
    const next = { ...guides, [axis]: [...new Set(list)].sort((a, b) => a - b) };
    onguides(next);
  }

  // ---- Drag (new from a ruler, or an existing guide) ----------------------------------------------

  interface Drag {
    axis: Axis;
    /** The guide's value when moving one; null for a new guide. */
    from: number | null;
    origin: DOMRect;
  }
  let drag: Drag | null = null;
  /** The guide being dragged: its position, or null while over the ruler (drop = delete / cancel). */
  let pending = $state<{ axis: Axis; from: number | null; pos: number | null } | null>(null);

  const frames = new FrameQueue<{ x: number; y: number }>((p) => {
    const d = drag;
    if (!d) return;
    const over = d.axis === "x" ? p.x < d.origin.left : p.y < d.origin.top;
    const raw = d.axis === "x" ? (p.x - d.origin.left) / scale : (p.y - d.origin.top) / scale;
    pending = { axis: d.axis, from: d.from, pos: over ? null : clampPos(d.axis, raw) };
  });

  function begin(e: PointerEvent, axis: Axis, from: number | null) {
    if (e.button !== 0 || !root || scale <= 0) return;
    if (from === null && guides[axis].length >= SCREEN_GUIDES_MAX) return;
    e.preventDefault();
    e.stopPropagation();
    try {
      (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    } catch {
      // Synthetic pointers can't be captured.
    }
    drag = { axis, from, origin: root.getBoundingClientRect() };
    pending = { axis, from, pos: from };
  }

  function onmove(e: PointerEvent) {
    if (drag) frames.push({ x: e.clientX, y: e.clientY });
  }

  function end(e: PointerEvent) {
    if (!drag) return;
    frames.push({ x: e.clientX, y: e.clientY });
    frames.flush();
    const p = pending;
    drag = null;
    pending = null;
    if (!p) return;
    const list = guides[p.axis].filter((v) => v !== p.from);
    if (p.pos === null) {
      if (p.from !== null) set(p.axis, list); // dropped on the ruler: delete
    } else if (p.pos !== p.from) {
      set(p.axis, [...list, p.pos]);
    }
  }

  $effect(() => () => frames.cancel());

  // ---- Typed position ------------------------------------------------------------------------------

  let editing = $state<{ axis: Axis; from: number; value: string } | null>(null);

  async function edit(axis: Axis, from: number) {
    editing = { axis, from, value: String(from) };
    await tick();
    editInput?.focus();
    editInput?.select();
  }

  function applyEdit() {
    const ed = editing;
    if (!ed) return;
    const n = Number(ed.value.trim());
    if (ed.value.trim() === "" || !Number.isFinite(n)) return;
    editing = null;
    const pos = clampPos(ed.axis, n);
    if (pos !== ed.from) set(ed.axis, [...guides[ed.axis].filter((v) => v !== ed.from), pos]);
  }

  function onEditKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      applyEdit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      editing = null;
    }
  }

  // ---- Keyboard ------------------------------------------------------------------------------------

  function onGuideKey(e: KeyboardEvent, axis: Axis, v: number) {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const step = e.shiftKey ? 10 : 1;
    const delta =
      axis === "x"
        ? { ArrowLeft: -step, ArrowRight: step }[e.key]
        : { ArrowUp: -step, ArrowDown: step }[e.key];
    if (delta !== undefined) {
      e.preventDefault();
      const pos = clampPos(axis, v + delta);
      set(axis, [...guides[axis].filter((g) => g !== v), pos]);
      void tick().then(() => root?.querySelector<HTMLElement>(`[data-guide="${axis}:${pos}"]`)?.focus());
    } else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      set(axis, guides[axis].filter((g) => g !== v));
    } else if (e.key === "Enter") {
      e.preventDefault();
      void edit(axis, v);
    }
  }

  function onRulerKey(e: KeyboardEvent, axis: Axis) {
    if (e.key !== "Enter" && e.key !== " ") return;
    e.preventDefault();
    if (guides[axis].length >= SCREEN_GUIDES_MAX) return;
    let pos = Math.round(sizeOf(axis) / 2);
    while (guides[axis].includes(pos)) pos += 10;
    set(axis, [...guides[axis], pos]);
    void edit(axis, pos);
  }

  const at = (axis: Axis, v: number) => (axis === "x" ? `translateX(${v * scale}px)` : `translateY(${v * scale}px)`);
</script>

<div class="rulers" bind:this={root}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="ruler top vf"
    role="button"
    tabindex="0"
    aria-label="Horizontal guides: drag down from here, or press Enter to add one"
    style:width="{screen.w * scale}px"
    onpointerdown={(e) => begin(e, "y", null)}
    onpointermove={onmove}
    onpointerup={end}
    onpointercancel={end}
    onkeydown={(e) => onRulerKey(e, "y")}
  >
    <svg width={screen.w * scale} height={T} aria-hidden="true">
      <path d={top.path} />
      {#each top.labels as v (v)}
        <text x={Math.round(v * scale) + 3} y="10">{v}</text>
      {/each}
    </svg>
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="ruler left vf"
    role="button"
    tabindex="0"
    aria-label="Vertical guides: drag right from here, or press Enter to add one"
    style:height="{screen.h * scale}px"
    onpointerdown={(e) => begin(e, "x", null)}
    onpointermove={onmove}
    onpointerup={end}
    onpointercancel={end}
    onkeydown={(e) => onRulerKey(e, "x")}
  >
    <svg width={T} height={screen.h * scale} aria-hidden="true">
      <path d={left.path} />
      {#each left.labels as v (v)}
        <text transform="translate(10 {Math.round(v * scale) + 3}) rotate(90)">{v}</text>
      {/each}
    </svg>
  </div>
  <span class="corner" aria-hidden="true"></span>

  {#each AXES as axis (axis)}
    {#each guides[axis] as v (v)}
      <!-- The dragged guide stays in the DOM (it holds the pointer capture), just invisible. -->
      <div
          class="guide {axis} vf"
          class:lifted={pending !== null && pending.axis === axis && pending.from === v}
          role="slider"
          tabindex="0"
          data-guide="{axis}:{v}"
          aria-label="{label(axis)} guide"
          aria-orientation={axis === "x" ? "horizontal" : "vertical"}
          aria-valuenow={v}
          aria-valuemin={-sizeOf(axis)}
          aria-valuemax={2 * sizeOf(axis)}
          aria-valuetext="{v} px"
          style:transform={at(axis, v)}
          onpointerdown={(e) => begin(e, axis, v)}
          onpointermove={onmove}
          onpointerup={end}
          onpointercancel={end}
          ondblclick={() => void edit(axis, v)}
          onkeydown={(e) => onGuideKey(e, axis, v)}
        ></div>
    {/each}
  {/each}

  {#if pending && pending.pos !== null}
    <div class="guide {pending.axis} moving" aria-hidden="true" style:transform={at(pending.axis, pending.pos)}>
      <span class="pos mono">{pending.pos}</span>
    </div>
  {/if}

  {#if editing}
    <span class="edit vf vf-within" style:transform={at(editing.axis, editing.from)} class:x={editing.axis === "x"}>
      <input
        bind:this={editInput}
        bind:value={editing.value}
        class="mono"
        type="text"
        inputmode="numeric"
        aria-label="{label(editing.axis)} guide position in px"
        autocomplete="off"
        spellcheck="false"
        maxlength="6"
        onkeydown={onEditKey}
        onblur={() => (editing = null)}
      />
    </span>
  {/if}
</div>

<style>
  .rulers {
    position: absolute;
    inset: 0;
    z-index: 2;
    pointer-events: none;
  }
  .ruler,
  .corner {
    position: absolute;
    background: var(--surface-1);
    pointer-events: auto;
  }
  .ruler {
    --vf-radius: 0px;
    overflow: hidden;
  }
  .ruler.top {
    left: 0;
    bottom: 100%;
    height: 20px;
    box-shadow: inset 0 0 0 1px var(--line);
    cursor: row-resize;
  }
  .ruler.left {
    top: 0;
    right: 100%;
    width: 20px;
    box-shadow: inset 0 0 0 1px var(--line);
    cursor: col-resize;
  }
  .corner {
    right: 100%;
    bottom: 100%;
    width: 20px;
    height: 20px;
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .ruler svg {
    display: block;
  }
  .ruler path {
    fill: none;
    stroke: var(--line-strong);
    stroke-width: 1;
  }
  .ruler text {
    fill: var(--text-muted);
    font-family: var(--font-mono);
    font-size: var(--fs-label);
  }

  .guide {
    --vf-radius: 0px;
    position: absolute;
    top: 0;
    left: 0;
    pointer-events: auto;
  }
  /* A 7 px hit strip with the 1 px line in its middle. */
  .guide.x {
    width: 7px;
    height: 100%;
    margin-left: -3px;
    cursor: col-resize;
  }
  .guide.y {
    width: 100%;
    height: 7px;
    margin-top: -3px;
    cursor: row-resize;
  }
  .guide::before {
    content: "";
    position: absolute;
    background: var(--accent);
    opacity: 0.85;
  }
  .guide.x::before {
    top: 0;
    bottom: 0;
    left: 3px;
    width: 1px;
  }
  .guide.y::before {
    left: 0;
    right: 0;
    top: 3px;
    height: 1px;
  }
  .guide.moving {
    pointer-events: none;
  }
  .guide.lifted {
    opacity: 0;
  }
  .guide.moving::before {
    opacity: 1;
  }
  .pos {
    position: absolute;
    top: 4px;
    left: 6px;
    padding: 0 3px;
    border-radius: 3px;
    background: var(--accent);
    color: var(--accent-ink);
    font-size: var(--fs-xs);
    line-height: 15px;
  }
  .edit {
    --vf-radius: var(--r-control);
    position: absolute;
    top: 4px;
    left: 6px;
    display: inline-flex;
    border-radius: var(--r-control);
    pointer-events: auto;
  }
  .edit:not(.x) {
    top: 6px;
    left: 4px;
  }
  .edit input {
    width: calc(7ch + 2 * var(--s-2));
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--accent);
    color: var(--text);
    font-size: var(--fs-sm);
  }
</style>
