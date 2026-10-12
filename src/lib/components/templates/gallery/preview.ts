// The gallery's hover / focus preview: ONE live, animated preview at a time, drawn by the
// renderer's own `compile` (through the variant thumbnails' renderThumb) into an inert shadow
// root over the card's static thumbnail, with the preset's sample data, playing its in-animation
// once. No iframe, no timers, no network beyond the preset files. Showing another releases the
// previous one; `hidePreview` releases it (the card lost hover and focus, or the gallery closed).
//
// Lazy chunk: the gallery imports it on the first hover or focus.

import { project } from "$lib/stores/project.svelte";
import type { BrandKit } from "$lib/types";
import { releaseThumb, renderThumb } from "../editor/thumbs";
import { loadPresetLook, sampleValues } from "./presets";

let current: HTMLElement | null = null;
/** The element the latest show is for (drawn or still loading). */
let target: HTMLElement | null = null;
/** Bumped by every show/hide: a load that finishes after a newer call draws nothing. */
let gen = 0;

/** Brand kit values as the shared library variables (they win over the variant's own). */
export function brandVars(brand: BrandKit | null | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  if (!brand) return out;
  const map: [string, string | null | undefined][] = [
    ["accent", brand.accent],
    ["secondary", brand.secondary],
    ["text", brand.text],
    ["plate", brand.plate],
    ["font-heading", brand.fontHeading],
    ["font-body", brand.fontBody],
  ];
  for (const [k, v] of map) if (v) out[k] = v;
  return out;
}

/** The element showing the preview now (tests). */
export const previewing = (): HTMLElement | null => current;

/**
 * Draws `presetId`'s `variant` into `el`, playing its in-animation, with the project's brand kit
 * (default: the open project's) over the variant's variables. Resolves true when drawn
 * (false: superseded by a newer call, or the preset has no such look).
 */
export async function showPreview(
  el: HTMLElement,
  presetId: string,
  variant: string,
  brand: BrandKit | null | undefined = project.current?.settings.brand,
): Promise<boolean> {
  const g = ++gen;
  target = el;
  if (current && current !== el) {
    releaseThumb(current);
    current = null;
  }
  const look = await loadPresetLook(presetId, variant);
  if (g !== gen || !look || !el.isConnected) return false;
  const values = await sampleValues(presetId, variant, look.fields);
  if (g !== gen || !el.isConnected) return false;
  const vars = { ...look.vars, ...brandVars(brand) };
  renderThumb(el, { html: look.html, css: look.css, vars }, look.fields, look.variables, { values, play: true });
  current = el;
  return true;
}

/** Releases the preview in `el` (any preview when omitted); a pending load for it draws nothing. */
export function hidePreview(el?: HTMLElement): void {
  if (el && target !== el) return;
  gen++;
  target = null;
  if (current) releaseThumb(current);
  current = null;
}
