// The gallery's hover preview reads a preset's look straight from the preset files the app is
// built with (`presets/templates/<id>/`): `template.json` (fields, variables, variants with their
// `vars`), the variant's `variants/<style>/template.html|css` (a preset without that folder: its
// root `template.html|css`) and optional sample data. Every file is its own lazy asset: nothing
// is fetched until a card is hovered or focused. Part of the preview chunk (./preview.ts).

import type { FieldDef, VariableDef } from "$lib/types";
import { thumbValue } from "../editor/thumbs";

const RAW = import.meta.glob<string>(
  [
    "/presets/templates/*/template.json",
    "/presets/templates/*/template.html",
    "/presets/templates/*/template.css",
    "/presets/templates/*/sample.json",
    "/presets/templates/*/variants/*/template.html",
    "/presets/templates/*/variants/*/template.css",
    "/presets/templates/*/variants/*/sample.json",
  ],
  { query: "?raw", import: "default" },
);

async function raw(path: string): Promise<string | null> {
  const load = RAW[path];
  if (!load) return null;
  try {
    return await load();
  } catch {
    return null;
  }
}

function json(text: string | null): unknown {
  if (text === null) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

/** One variant of a preset, ready for `renderThumb`. */
export interface PresetLook {
  fields: FieldDef[];
  variables: VariableDef[];
  html: string;
  css: string;
  /** The variant's variable values (its `vars`). */
  vars: Record<string, string>;
}

/** Strings only: a variant's `vars` or a sample's values as the renderer takes them. */
function strings(v: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  if (!isObj(v)) return out;
  for (const [k, x] of Object.entries(v)) {
    if (typeof x === "string") out[k] = x;
    else if (typeof x === "number" || typeof x === "boolean") out[k] = String(x);
    // A list value (rows) or a timer state: the field's JSON value.
    else if (x !== null && typeof x === "object") out[k] = JSON.stringify(x);
  }
  return out;
}

/** A preset's look for `variant`, or null when the preset (or its markup) isn't in the build. */
export async function loadPresetLook(presetId: string, variant: string): Promise<PresetLook | null> {
  const base = `/presets/templates/${presetId}`;
  const dir = `${base}/variants/${variant}`;
  const [meta, vHtml, vCss] = await Promise.all([
    raw(`${base}/template.json`).then(json),
    raw(`${dir}/template.html`),
    raw(`${dir}/template.css`),
  ]);
  if (!isObj(meta)) return null;
  const html = vHtml ?? (await raw(`${base}/template.html`));
  if (html === null) return null;
  const css = vHtml !== null ? (vCss ?? "") : ((await raw(`${base}/template.css`)) ?? "");
  const variants = Array.isArray(meta.variants) ? meta.variants : [];
  const v = variants.find((x: unknown) => isObj(x) && x.id === variant);
  return {
    fields: Array.isArray(meta.fields) ? (meta.fields as FieldDef[]) : [],
    variables: Array.isArray(meta.variables) ? (meta.variables as VariableDef[]) : [],
    html,
    css,
    vars: strings(isObj(v) ? v.vars : null),
  };
}

/** A preset's sample data alone (field key → value; {} without one). The template editor's
 *  preview takes its list rows from here. */
export async function presetSample(presetId: string, variant: string): Promise<Record<string, string>> {
  const base = `/presets/templates/${presetId}`;
  let sample = json(await raw(`${base}/variants/${variant}/sample.json`));
  if (!isObj(sample)) sample = json(await raw(`${base}/sample.json`));
  return strings(isObj(sample) && isObj(sample.fields) ? sample.fields : sample);
}

/**
 * The values a preview shows: the preset's sample data (`variants/<style>/sample.json`, else
 * `sample.json`: an object of field key → value; a `fields` object inside is accepted too), and
 * for every other field what a thumbnail shows (its default, else its label). This is the one
 * place sample data plugs in.
 */
export async function sampleValues(
  presetId: string,
  variant: string,
  fields: readonly FieldDef[],
): Promise<Record<string, string>> {
  const given = await presetSample(presetId, variant);
  const out: Record<string, string> = {};
  for (const f of fields) out[f.key] = given[f.key] ?? thumbValue(f);
  return out;
}
