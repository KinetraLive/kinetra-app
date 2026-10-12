// Gallery thumbnails: static `presets/thumbs/<presetId>.png` and, per style look,
// `<presetId>-<style>.webp` (lossy WebP with alpha) files shipped with the app UI only (generated
// by `pnpm shots:presets`; kept out of presets/templates/, which rust-embed puts into the binary).
// Each is its own asset, resolved lazily: nothing is fetched until the gallery asks for it, and a
// preset without a thumbnail resolves to null.

type Loader = () => Promise<string>;

const GLOB = import.meta.glob<string>("/presets/thumbs/*.{png,webp}", {
  eager: false,
  query: "?url",
  import: "default",
});

/**
 * Loaders keyed by thumbnail name (the file name without its extension). When both
 * `<name>.webp` and `<name>.png` exist, the WebP wins.
 */
export function thumbIndex(files: Record<string, Loader>): Map<string, Loader> {
  const out = new Map<string, Loader>();
  for (const [path, load] of Object.entries(files)) {
    const m = /\/([^/]+)\.(png|webp)$/.exec(path);
    if (!m) continue;
    if (m[2] === "png" && out.has(m[1])) continue;
    out.set(m[1], load);
  }
  return out;
}

const THUMBS = thumbIndex(GLOB);

/** URL of the thumbnail named `name` (`<presetId>` or `<presetId>-<style>`), or null when there is none. */
export async function thumbUrl(name: string): Promise<string | null> {
  const load = THUMBS.get(name);
  if (!load) return null;
  try {
    return await load();
  } catch {
    return null;
  }
}

/**
 * Thumbnail of one look of a preset: `presets/thumbs/<presetId>-<variant>` (.webp, else .png),
 * else the preset's own `<presetId>.png`, else null.
 */
export async function variantThumbUrl(presetId: string, variant: string): Promise<string | null> {
  return (variant ? await thumbUrl(`${presetId}-${variant}`) : null) ?? (await thumbUrl(presetId));
}
