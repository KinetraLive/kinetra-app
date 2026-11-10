/**
 * Menus and popovers that open from a scrolling column (the layout editor's Layers panel and
 * Inspector have `overflow: auto`, which clips anything positioned inside them):
 * `{@attach floating(() => anchorEl, { align: "end" })}` on the panel shows it in the browser's
 * top layer (Popover API, `popover="manual"`), so no scrolling ancestor clips it and no z-index
 * covers it, and places it with fixed coordinates next to its anchor (`place`). The panel stays
 * where it is in the DOM: focus order, keyboard handling and "press outside" checks
 * (`root.contains`) are unchanged. Its CSS must reset the popover defaults (`inset: auto;
 * margin: 0; border: 0`) and keep a fixed width.
 *
 * While open it is re-placed on window resize, on any scroll (a column scrolling under it) and
 * when its content changes (a menu's next step, sources arriving), coalesced to one placement
 * per frame. Nothing runs while closed: the listeners live with the element.
 */

export interface FloatOptions {
  /** Which edge lines up with the anchor's: "start" = left edges, "end" = right edges. */
  align?: "start" | "end";
}

export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** Space between the anchor and the panel, and between the panel and the window's edge (px). */
export const GAP = 4;
export const MARGIN = 8;

/**
 * Where the panel goes: below the anchor, or above it when it doesn't fit below and there is more
 * room above; its height capped to that room (`maxHeight`, else null); left aligned per `align`
 * and kept inside the window.
 */
export function place(
  anchor: Box,
  size: { width: number; height: number },
  view: { width: number; height: number },
  align: "start" | "end" = "start",
): { left: number; top: number; maxHeight: number | null } {
  const below = view.height - MARGIN - (anchor.bottom + GAP);
  const above = anchor.top - GAP - MARGIN;
  const down = size.height <= below || below >= above;
  const room = Math.max(0, Math.floor(down ? below : above));
  const height = Math.min(size.height, room);
  const top = down ? anchor.bottom + GAP : anchor.top - GAP - height;
  const wanted = align === "end" ? anchor.right - size.width : anchor.left;
  const left = Math.max(MARGIN, Math.min(wanted, view.width - MARGIN - size.width));
  return { left: Math.round(left), top: Math.round(top), maxHeight: size.height > room ? room : null };
}

export function floating(anchor: () => Element | null | undefined, opts: FloatOptions = {}) {
  return (el: HTMLElement) => {
    // Only where the Popover API exists: a `popover` element that can't be shown stays hidden.
    // Without it the panel's own `position: fixed` (+ z-index) still escapes the column's clip.
    if (typeof el.showPopover === "function") {
      el.setAttribute("popover", "manual");
      try {
        el.showPopover();
      } catch {
        // Detached already: nothing to show.
      }
    }

    const update = () => {
      const a = anchor();
      if (!a || !el.isConnected) return;
      el.style.maxHeight = "";
      const p = place(
        a.getBoundingClientRect(),
        { width: el.offsetWidth, height: el.offsetHeight },
        { width: window.innerWidth, height: window.innerHeight },
        opts.align,
      );
      el.style.left = `${p.left}px`;
      el.style.top = `${p.top}px`;
      if (p.maxHeight !== null) el.style.maxHeight = `${p.maxHeight}px`;
    };
    update();

    let frame = 0;
    const later = () => {
      if (!frame) frame = requestAnimationFrame(() => ((frame = 0), update()));
    };
    // Scrolls inside the panel itself (a long menu) don't move it.
    const onScroll = (e: Event) => {
      if (!(e.target instanceof Node && el.contains(e.target))) later();
    };
    window.addEventListener("resize", later);
    window.addEventListener("scroll", onScroll, { capture: true, passive: true });
    const content = typeof MutationObserver === "function" ? new MutationObserver(later) : null;
    content?.observe(el, { childList: true, subtree: true, characterData: true });

    return () => {
      if (frame) cancelAnimationFrame(frame);
      window.removeEventListener("resize", later);
      window.removeEventListener("scroll", onScroll, { capture: true });
      content?.disconnect();
    };
  };
}
