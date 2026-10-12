<!--
  TBar: the vertical T-bar of the studio column (lazy studio chunk). Bottom = 0 (Program), top = 1
  (Preview taken). Grabbing it begins a T-bar transition; moving sets the position (pointer moves
  are coalesced to one per frame, the wire to ≤ 30/s by TbarOps); releasing at the top commits the
  take, anywhere else aborts it (Program unchanged).

  Keyboard (role slider): ArrowUp/Right +5 %, ArrowDown/Left −5 %, PageUp/PageDown ±25 %, Home 0,
  End 100 %. The first key begins; reaching the top commits, the bottom aborts; Esc or leaving the
  slider aborts. The handle and fill move with transforms only; the server's position is shown
  when nobody holds the bar.

  Props:
    value: number | null     PreviewView.tbar (the server's position, null = no transition)
    disabled?: boolean
    ops: TbarOps
-->
<script lang="ts">
  import { TBAR_COMMIT, TBAR_PAGE, TBAR_STEP, clamp01, type TbarOps } from "./tbar";

  let {
    value,
    disabled = false,
    ops,
  }: { value: number | null; disabled?: boolean; ops: TbarOps } = $props();

  let track: HTMLDivElement | undefined = $state();
  /** The position while held (pointer or keyboard); null when released. */
  let local = $state<number | null>(null);
  let dragging = $state(false);
  const shown = $derived(local ?? value ?? 0);
  const pct = $derived(Math.round(shown * 100));

  let raf = 0;
  let want = 0;

  function set(p: number) {
    local = clamp01(p);
    ops.set(local);
  }

  function begin() {
    // No transition: the server runs the saved default.
    ops.begin(null);
    local = value ?? 0;
  }

  function release() {
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    const p = local ?? 0;
    dragging = false;
    local = null;
    void ops.end(p >= TBAR_COMMIT);
  }

  function at(e: PointerEvent): number {
    const r = track?.getBoundingClientRect();
    return r && r.height > 0 ? clamp01((r.bottom - e.clientY) / r.height) : 0;
  }

  function onpointerdown(e: PointerEvent) {
    if (disabled || e.button !== 0 || !track) return;
    e.preventDefault();
    track.focus();
    track.setPointerCapture?.(e.pointerId);
    dragging = true;
    begin();
    set(at(e));
  }

  function onpointermove(e: PointerEvent) {
    if (!dragging) return;
    want = at(e);
    raf ||= requestAnimationFrame(() => {
      raf = 0;
      if (dragging) set(want);
    });
  }

  function onpointerup(e: PointerEvent) {
    if (!dragging) return;
    if (raf) set(at(e));
    release();
  }

  function onkeydown(e: KeyboardEvent) {
    if (disabled || e.ctrlKey || e.altKey || e.metaKey) return;
    const cur = local ?? value ?? 0;
    let next: number;
    switch (e.key) {
      case "ArrowUp":
      case "ArrowRight":
        next = cur + TBAR_STEP;
        break;
      case "ArrowDown":
      case "ArrowLeft":
        next = cur - TBAR_STEP;
        break;
      case "PageUp":
        next = cur + TBAR_PAGE;
        break;
      case "PageDown":
        next = cur - TBAR_PAGE;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = 1;
        break;
      case "Escape":
        if (local === null || e.shiftKey) return;
        e.preventDefault();
        e.stopPropagation();
        local = 0;
        release();
        return;
      default:
        return;
    }
    e.preventDefault();
    // Snap to whole steps so 20 presses land exactly on 1.
    next = clamp01(Math.round(next * 100) / 100);
    if (local === null) {
      if (next <= 0) return;
      begin();
    }
    set(next);
    if (next >= TBAR_COMMIT || next <= 0) release();
  }

  function onblur() {
    if (local !== null && !dragging) release();
  }
</script>

<div
  class="tbar vf"
  class:dragging
  class:held={local !== null}
  role="slider"
  tabindex={disabled ? -1 : 0}
  aria-label="T-bar"
  aria-orientation="vertical"
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={pct}
  aria-valuetext="{pct} %"
  aria-disabled={disabled}
  bind:this={track}
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={release}
  {onkeydown}
  {onblur}
  style:--p={shown}
>
  <span class="fill" aria-hidden="true"></span>
  <span class="slide" aria-hidden="true"><span class="handle"></span></span>
</div>
<span class="pos" aria-hidden="true">{pct}%</span>

<style>
  .tbar {
    --vf-radius: var(--r-control);
    --handle: 20px;
    position: relative;
    flex: 1;
    width: 44px;
    min-height: 64px;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    cursor: grab;
    touch-action: none;
    user-select: none;
  }
  .tbar.dragging {
    cursor: grabbing;
  }
  .tbar[aria-disabled="true"] {
    cursor: default;
    opacity: 0.5;
  }
  .tbar:focus-visible {
    outline: none;
  }
  .fill {
    position: absolute;
    inset: 1px;
    border-radius: calc(var(--r-control) - 1px);
    background: var(--accent-soft);
    transform: scaleY(var(--p));
    transform-origin: bottom;
  }
  .slide {
    position: absolute;
    top: 0;
    right: 0;
    left: 0;
    height: calc(100% - var(--handle));
    transform: translateY(calc((1 - var(--p)) * 100%));
  }
  .handle {
    position: absolute;
    top: 0;
    right: -6px;
    left: -6px;
    height: var(--handle);
    border-radius: var(--r-control);
    background: var(--text);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .held .handle {
    background: var(--accent);
  }
  /* The server's position settles smoothly; a held bar follows the pointer exactly. */
  .tbar:not(.held) .fill,
  .tbar:not(.held) .slide {
    transition: transform var(--dur) var(--ease);
  }
  .pos {
    color: var(--text-muted);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }
  @media (prefers-reduced-motion: reduce) {
    .tbar:not(.held) .fill,
    .tbar:not(.held) .slide {
      transition: none;
    }
  }
</style>
