// A coarse clock for relative times ("updated 4 s ago"). A component that shows them holds the
// clock while it does (`$effect(() => hold())`); `now()` read in its template then re-reads once a
// second, aligned to the second, only while someone holds it and the page is visible. Nothing is
// scheduled otherwise. (Plain $state rather than svelte/reactivity's createSubscriber: importing that
// here would split it out of the startup chunk the theme store shares it in.)

let tick = $state(0);
let holders = 0;
let timer: ReturnType<typeof setTimeout> | undefined;

const hidden = () => document.visibilityState === "hidden";

function schedule() {
  if (hidden()) return;
  timer = setTimeout(fire, 1000 - (Date.now() % 1000) + 5);
}

function fire() {
  timer = undefined;
  tick++;
  schedule();
}

function onVisibility() {
  clearTimeout(timer);
  timer = undefined;
  if (!hidden()) fire();
}

/** Keeps the clock running; returns the release (an effect's teardown). */
export function hold(): () => void {
  if (holders++ === 0) {
    document.addEventListener("visibilitychange", onVisibility);
    schedule();
  }
  return () => {
    if (--holders > 0) return;
    clearTimeout(timer);
    timer = undefined;
    document.removeEventListener("visibilitychange", onVisibility);
  };
}

/** Date.now(), re-read every second while the clock is held. */
export function now(): number {
  void tick;
  return Date.now();
}
