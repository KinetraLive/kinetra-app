// Live screen thumbnails for a grid: which cards may hold a live iframe right now, and each
// thumbnail box's size. Shared by the dashboard grid, the /screens grid and the Control grid.
//
// Performance (hard rule): every live thumbnail is a cross-site iframe, i.e. a WebView2 renderer
// process. A card's iframe exists only while its thumbnail intersects the viewport AND the window
// is visible AND the server runs, and only for the first `cap` such cards in grid order (`first`,
// e.g. Control's selected screen, counts first). Every other card shows the checkerboard, with
// `note()` explaining why. Iframes are removed from the DOM, never hidden. One
// IntersectionObserver and one ResizeObserver per grid; nothing runs at rest.
//
//   const thumbs = new LiveThumbs({ order: () => ids, running: () => running });
//   $effect(() => () => thumbs.destroy());
//   <div {@attach thumbs.track(id)}>{#if thumbs.live.has(id)}<iframe …/>{/if}</div>

import type { Attachment } from "svelte/attachments";
import { MAX_LIVE_THUMBNAILS, pickLive } from "./pick";

export interface LiveThumbsOptions {
  /** Screen ids in grid order. */
  order: () => readonly string[];
  /** The output server runs (no server: nothing to load). */
  running: () => boolean;
  /** Most live iframes at once. */
  cap?: number;
  /** A screen that counts first for the cap (if it is visible), e.g. the selected one. */
  first?: () => string | null;
}

export interface Box {
  w: number;
  h: number;
}

const NO_BOX: Box = { w: 0, h: 0 };

export class LiveThumbs {
  /** Ids whose thumbnail box intersects the viewport. Replaced wholesale per observer callback. */
  visible = $state.raw<ReadonlySet<string>>(new Set());
  /** Thumbnail box size per id. */
  boxes = $state.raw<Readonly<Record<string, Box>>>({});
  pageVisible = $state(typeof document === "undefined" || document.visibilityState === "visible");

  readonly #opts: LiveThumbsOptions;
  readonly #io: IntersectionObserver;
  readonly #ro: ResizeObserver;
  readonly #trackers = new Map<string, Attachment<HTMLElement>>();
  readonly #onVisibility = () => {
    this.pageVisible = document.visibilityState === "visible";
  };

  /** Ids that get a live iframe now. */
  live = $derived.by((): ReadonlySet<string> => {
    if (!this.#opts.running() || !this.pageVisible) return new Set();
    const order = this.#opts.order();
    const first = this.#opts.first?.() ?? null;
    const ordered = first && order.includes(first) ? [first, ...order.filter((id) => id !== first)] : order;
    return pickLive([...ordered], this.visible, this.#opts.cap ?? MAX_LIVE_THUMBNAILS);
  });

  constructor(opts: LiveThumbsOptions) {
    this.#opts = opts;
    const idOf = (el: Element) => (el as HTMLElement).dataset.screenId ?? "";
    this.#io = new IntersectionObserver((entries) => {
      const next = new Set(this.visible);
      for (const e of entries) {
        if (e.isIntersecting) next.add(idOf(e.target));
        else next.delete(idOf(e.target));
      }
      this.visible = next;
    });
    this.#ro = new ResizeObserver((entries) => {
      const next = { ...this.boxes };
      for (const e of entries) next[idOf(e.target)] = { w: e.contentRect.width, h: e.contentRect.height };
      this.boxes = next;
    });
    if (typeof document !== "undefined") document.addEventListener("visibilitychange", this.#onVisibility);
  }

  /** Attachment for a card's thumbnail box; one stable function per id, so re-renders never
   *  re-observe. */
  track(id: string): Attachment<HTMLElement> {
    let t = this.#trackers.get(id);
    if (!t) {
      t = (el) => {
        el.dataset.screenId = id;
        this.#io.observe(el);
        this.#ro.observe(el);
        return () => {
          this.#io.unobserve(el);
          this.#ro.unobserve(el);
          this.#trackers.delete(id);
          if (this.visible.has(id)) {
            const next = new Set(this.visible);
            next.delete(id);
            this.visible = next;
          }
        };
      };
      this.#trackers.set(id, t);
    }
    return t;
  }

  box(id: string): Box {
    return this.boxes[id] ?? NO_BOX;
  }

  /** Why a card has no live iframe, or null (live, or simply off-screen). */
  note(id: string): string | null {
    if (this.live.has(id)) return null;
    if (!this.#opts.running()) return "Server not running";
    if (this.pageVisible && this.visible.has(id)) return "Preview paused";
    return null;
  }

  destroy(): void {
    this.#io.disconnect();
    this.#ro.disconnect();
    this.#trackers.clear();
    if (typeof document !== "undefined") document.removeEventListener("visibilitychange", this.#onVisibility);
  }
}
