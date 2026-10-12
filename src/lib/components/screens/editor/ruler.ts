// Ruler and grid spacing of the layout editor (lazy: part of the overlays chunk). Depends only on
// the zoom (scale = CSS px per screen px), so rulers and grid re-render on resize / zoom only.

const NICE = [1, 2, 5];

/** The smallest 1-2-5 step (screen px) whose labels are at least `minCss` CSS px apart. */
export function rulerStep(scale: number, minCss = 56): number {
  if (!(scale > 0)) return 0;
  for (let p = 1; p <= 1e6; p *= 10) {
    for (const n of NICE) if (n * p * scale >= minCss) return n * p;
  }
  return 1e6;
}

export interface RulerTicks {
  /** Label positions (screen px), multiples of `step`. */
  labels: number[];
  /** One SVG path of all tick marks across the ruler (CSS px), `depth` tall at the major ones. */
  path: string;
}

/**
 * Ticks of a ruler `length` screen px long: majors at every `rulerStep`, minor ticks at 1/10 (or
 * 1/5, 1/2) when they are at least 4 CSS px apart. Horizontal ruler (`vertical` = false): ticks
 * hang from the bottom edge at `thickness`; a vertical ruler mirrors it.
 */
export function rulerTicks(length: number, scale: number, thickness: number, vertical = false): RulerTicks {
  const step = rulerStep(scale);
  if (step <= 0 || !(length > 0)) return { labels: [], path: "" };
  const div = [10, 5, 2].find((d) => (step / d) * scale >= 4 && Number.isInteger(step / d)) ?? 1;
  const minor = step / div;
  const labels: number[] = [];
  let path = "";
  for (let i = 0, v = 0; v <= length; i++, v = i * minor) {
    const major = i % div === 0;
    if (major) labels.push(v);
    const half = div % 2 === 0 && i % (div / 2) === 0;
    const depth = major ? thickness * 0.5 : half ? thickness * 0.3 : thickness * 0.18;
    const at = Math.round(v * scale) + 0.5;
    path += vertical ? `M${thickness} ${at}h${-depth}` : `M${at} ${thickness}v${-depth}`;
  }
  return { labels, path };
}

/** The grid spacing to draw: `size` doubled until cells are at least `minCss` CSS px. */
export function visibleGrid(size: number, scale: number, minCss = 6): number {
  if (!(size > 0) || !(scale > 0)) return 0;
  let s = size;
  while (s * scale < minCss) s *= 2;
  return s;
}
