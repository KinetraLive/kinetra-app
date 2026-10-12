<!--
  PanelDivider: the vertical line between Control's stage and its right panel; drag it (or focus
  it and use ← / →, 16 px a step; Home / End for the smallest / largest) to resize the panel,
  280–640 px (panel.ts).

  Cheap while dragging: pointer moves are coalesced to one write per animation frame, and that
  write is only the `--panel-w` custom property on `target` (and aria-valuenow); nothing else
  re-renders. The width is saved once, on release (keys: 400 ms after the last one).

  Props:
    target: HTMLElement | undefined   the element whose `--panel-w` sizes the panel's column
    panel: HTMLElement | undefined    the panel (its width is the start of a drag)
    width: number | null              the saved width (null = the default)
    controls: string                  the panel's id (aria-controls)
    oncommit: (width: number) => void
-->
<script lang="ts">
  import { PANEL_WIDTH, dragWidth, keyWidth } from "./panel";

  let {
    target,
    panel,
    width,
    controls,
    oncommit,
  }: {
    target: HTMLElement | undefined;
    panel: HTMLElement | undefined;
    width: number | null;
    controls: string;
    oncommit: (width: number) => void;
  } = $props();

  let el: HTMLDivElement | undefined = $state();
  /** The default width as measured (shown while nothing is saved). */
  let measured = $state<number | null>(null);
  let dragging = $state(false);

  // Not reactive: the drag's working values.
  let shown: number | null = null;
  let start = { width: 0, x: 0 };
  let frame = 0;
  let keyTimer: ReturnType<typeof setTimeout> | undefined;

  const valueNow = $derived(Math.round(width ?? measured ?? PANEL_WIDTH.min));

  function measure(): number {
    return panel ? Math.round(panel.getBoundingClientRect().width) : (width ?? PANEL_WIDTH.min);
  }

  $effect(() => {
    if (width === null && panel) measured = measure();
  });

  /** The width now: what the drag or keys last showed, else the saved one, else the panel's. */
  const current = () => shown ?? width ?? measure();

  function apply() {
    frame = 0;
    if (shown === null) return;
    target?.style.setProperty("--panel-w", `${shown}px`);
    el?.setAttribute("aria-valuenow", String(shown));
  }

  function show(w: number) {
    shown = w;
    if (!frame) frame = requestAnimationFrame(apply);
  }

  function commit() {
    if (frame) cancelAnimationFrame(frame);
    apply();
    const w = shown;
    shown = null;
    if (w !== null && w !== width) oncommit(w);
  }

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    el?.focus();
    el?.setPointerCapture?.(e.pointerId);
    start = { width: current(), x: e.clientX };
    dragging = true;
  }

  function onpointermove(e: PointerEvent) {
    if (dragging) show(dragWidth(start.width, start.x, e.clientX));
  }

  function onpointerup(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    el?.releasePointerCapture?.(e.pointerId);
    commit();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const w = keyWidth(e.key, current());
    if (w === null) return;
    e.preventDefault();
    show(w);
    clearTimeout(keyTimer);
    keyTimer = setTimeout(commit, 400);
  }

  function onblur() {
    if (keyTimer === undefined) return;
    clearTimeout(keyTimer);
    keyTimer = undefined;
    commit();
  }

  $effect(
    () => () => {
      if (frame) cancelAnimationFrame(frame);
      if (keyTimer !== undefined) {
        clearTimeout(keyTimer);
        commit();
      }
    },
  );
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  bind:this={el}
  class="divider"
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-label="Resize the side panel"
  aria-controls={controls}
  aria-valuenow={valueNow}
  aria-valuemin={PANEL_WIDTH.min}
  aria-valuemax={PANEL_WIDTH.max}
  tabindex="0"
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={onpointerup}
  {onkeydown}
  {onblur}
></div>

<style>
  .divider {
    position: relative;
    z-index: 1;
    width: 1px;
    background: var(--line);
    cursor: col-resize;
    touch-action: none;
  }
  /* A wider grab area than the 1px line. */
  .divider::before {
    content: "";
    position: absolute;
    inset: 0 -4px;
  }
  .divider:hover,
  .divider.dragging,
  .divider:focus-visible {
    background: var(--accent);
  }
  .divider:focus-visible {
    outline-offset: 0;
  }
</style>
