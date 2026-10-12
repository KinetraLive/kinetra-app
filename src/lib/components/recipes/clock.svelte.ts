// A wall clock for the recipe panels that ticks once per second, aligned to the second, and ONLY
// while someone watches it. Ref-counted: the first watcher starts one timeout chain, the last
// unwatch stops it. Nothing runs at rest.
//
//   // in a panel: tick only while it is on screen and the window is visible
//   watchClockWhileVisible(() => root);
//   const now = $derived(clock.now);

class Clock {
  /** Local epoch ms of the last tick. */
  now = $state(Date.now());

  #watchers = 0;
  #timer: ReturnType<typeof setTimeout> | null = null;

  /** Starts ticking (if not already); returns the release. */
  watch(): () => void {
    if (this.#watchers++ === 0) this.#tick();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      if (--this.#watchers === 0 && this.#timer !== null) {
        clearTimeout(this.#timer);
        this.#timer = null;
      }
    };
  }

  get watching(): boolean {
    return this.#watchers > 0;
  }

  #tick = () => {
    const t = Date.now();
    this.now = t;
    // Wake just after the next whole second.
    this.#timer = setTimeout(this.#tick, 1000 - (t % 1000) + 5);
  };
}

export const clock = new Clock();

/**
 * Watches the clock while `el()` is on screen (IntersectionObserver) and the document is visible.
 * Call during component init. Without IntersectionObserver the element counts as on screen.
 */
export function watchClockWhileVisible(el: () => Element | undefined): void {
  const hasIO = typeof IntersectionObserver !== "undefined";
  let onScreen = $state(!hasIO);
  let pageVisible = $state(typeof document === "undefined" || document.visibilityState !== "hidden");

  $effect(() => {
    const node = el();
    if (!node || !hasIO) return;
    const io = new IntersectionObserver((entries) => {
      const e = entries.at(-1);
      if (e) onScreen = e.isIntersecting;
    });
    io.observe(node);
    return () => io.disconnect();
  });

  $effect(() => {
    const sync = () => (pageVisible = document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", sync);
    return () => document.removeEventListener("visibilitychange", sync);
  });

  $effect(() => {
    if (onScreen && pageVisible) return clock.watch();
  });
}
