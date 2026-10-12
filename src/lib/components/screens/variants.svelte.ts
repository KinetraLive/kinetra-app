// Template variants as the "using" side sees them: picker options ("Full (default)"), the palette
// commands "Switch <layer> to <variant>", and a small cache of variant names for the places that
// only know a layer (Control, the flow editors). Loaded lazily by those places: Control's
// expanded layer area and palette commands, the flow inspector and linear editor; the screen
// editor's Inspector uses the template docs it already has.
//
// The cache asks `get_template` once per template (and `get_screen` once per screen, for a
// layer's template). Control passes the template's `updatedAt` as a stamp, so a renamed or added
// variant is fetched again; the flow editors call `reset()` when they open.

import { untrack } from "svelte";
import { registerCommands, type Command, type KeyScope, type Section } from "$lib/commands/registry.svelte";
import { getScreen } from "$lib/api/screens";
import { getTemplate } from "$lib/api/templates";
import { t } from "$lib/i18n";
import type { ScreenDoc, TemplateDoc, VariantTransition } from "$lib/types";

export type VariantOpt = { value: string; label: string };

/** A template's variants in tab order, the default marked: "Full (default)". */
export function variantOptions(t: {
  defaultVariant: string;
  variants: readonly { id: string; name: string }[];
}): VariantOpt[] {
  return t.variants.map((v) => ({ value: v.id, label: v.id === t.defaultVariant ? `${v.name} (default)` : v.name }));
}

export const TRANSITIONS: { value: VariantTransition; label: string }[] = [
  { value: "crossfade", label: "Crossfade" },
  { value: "outIn", label: "Out and in" },
];

/** A layer with more than one look, as the palette lists it. */
export interface VariantLayer {
  screenId: string;
  layerId: string;
  name: string;
  /** The variant shown now (no command for it). */
  current: string;
  variants: readonly { id: string; name: string }[];
}

/** Where variant commands are listed and their keys work (Control: live; the layout editor). */
export interface VariantWhere {
  section: Exclude<Section, "recent">;
  keyScope: KeyScope;
}

/** A "Set variant…" pick option's id: the layer and the variant. */
const optionId = (l: VariantLayer, variant: string) => JSON.stringify([l.layerId, variant]);

/**
 * "Switch Lower third to Compact" for every other variant of every given layer, and one "Set
 * variant…" (`variant.set`) whose pick step lists every "Layer → Variant" pair. A pick step can't
 * nest (layer, then variant), so the pairs are one list: one query finds both names ("low comp"),
 * and the main list gets one command instead of one per layer.
 */
export function variantCommands(
  layers: readonly VariantLayer[],
  where: VariantWhere,
  run: (l: VariantLayer, variant: string) => void | Promise<void>,
): Command[] {
  const out: Command[] = [];
  const switchable = layers.filter((l) => l.variants.length >= 2);
  if (switchable.length > 0) {
    out.push({
      id: "variant.set",
      title: "Set variant…",
      section: where.section,
      keyScope: where.keyScope,
      next: {
        kind: "pick",
        placeholder: "Choose a layer and a variant",
        options: () =>
          switchable.flatMap((l) =>
            l.variants
              .filter((v) => v.id !== l.current)
              .map((v) => ({ id: optionId(l, v.id), title: t("{layer} → {variant}", { layer: l.name, variant: v.name }) })),
          ),
        run: (id) => {
          let pick: unknown;
          try {
            pick = JSON.parse(id);
          } catch {
            return;
          }
          if (!Array.isArray(pick)) return;
          const l = switchable.find((x) => x.layerId === pick[0]);
          if (l && l.variants.some((v) => v.id === pick[1])) return run(l, String(pick[1]));
        },
      },
    });
  }
  for (const l of switchable) {
    for (const v of l.variants) {
      if (v.id === l.current) continue;
      out.push({
        id: `variant:${l.screenId}/${l.layerId}/${v.id}`,
        title: "Switch {layer} to {variant}",
        params: () => ({ layer: l.name, variant: v.name }),
        section: where.section,
        keyScope: where.keyScope,
        run: () => run(l, v.id),
      });
    }
  }
  return out;
}

/** Same layers, names and variants: no need to register the commands again. */
export const variantsKey = (layers: readonly VariantLayer[]): string =>
  JSON.stringify(layers.map((l) => [l.layerId, l.name, l.current, l.variants.map((v) => [v.id, v.name])]));

/**
 * Registers the commands for `layers()` and keeps them current (re-registered only when a layer,
 * its name, its variant or the variants change); returns the function that removes them.
 */
export function watchVariantCommands(
  layers: () => VariantLayer[],
  where: VariantWhere,
  run: (l: VariantLayer, variant: string) => void | Promise<void>,
): () => void {
  let off = () => {};
  let last = "";
  const stop = $effect.root(() => {
    $effect(() => {
      const list = layers();
      const key = variantsKey(list);
      if (key === last) return;
      last = key;
      untrack(() => {
        off();
        off = registerCommands(variantCommands(list, where, run));
      });
    });
  });
  return () => {
    stop();
    off();
  };
}

/** The layout editor's layers with several variants (its loaded template docs). */
export function docVariantLayers(
  screenId: string,
  doc: ScreenDoc | null,
  templates: ReadonlyMap<string, TemplateDoc>,
): VariantLayer[] {
  const out: VariantLayer[] = [];
  for (const l of doc?.layers ?? []) {
    if (l.kind === "media") continue;
    const t = templates.get(l.templateId);
    if (t && t.variants.length > 1)
      out.push({ screenId, layerId: l.id, name: l.name, current: l.variant || t.defaultVariant, variants: t.variants });
  }
  return out;
}

interface Loaded {
  defaultVariant: string;
  variants: { id: string; name: string }[];
}

class VariantCache {
  /** `templateId@stamp` → its variants; null while loading or when it failed. */
  #templates = $state.raw<Record<string, Loaded | null>>({});
  /** `screenId` → layer id → template id; null while loading or when it failed. */
  #screens = $state.raw<Record<string, Record<string, string> | null>>({});

  /** The variants of a template (null until loaded). `stamp`: refetch when it changes. */
  template(templateId: string, stamp = ""): Loaded | null {
    if (!templateId) return null;
    const key = `${templateId}@${stamp}`;
    if (key in this.#templates) return this.#templates[key];
    queueMicrotask(() => this.#loadTemplate(key, templateId));
    return null;
  }

  /** Picker options for a template's variants (empty until loaded). */
  options(templateId: string, stamp = ""): VariantOpt[] {
    const t = this.template(templateId, stamp);
    return t ? variantOptions(t) : [];
  }

  /** The template of a screen's layer ("" until loaded or when it doesn't exist). */
  templateOf(screenId: string, layerId: string): string {
    if (!screenId || !layerId) return "";
    if (!(screenId in this.#screens)) {
      queueMicrotask(() => this.#loadScreen(screenId));
      return "";
    }
    return this.#screens[screenId]?.[layerId] ?? "";
  }

  /** Picker options for the variants of a screen's layer (empty until loaded). */
  ofLayer(screenId: string, layerId: string): VariantOpt[] {
    return this.options(this.templateOf(screenId, layerId));
  }

  /**
   * A layer's variant picker: its variants with `current` kept ("Missing (id)" once loaded without
   * it); while loading, just `current`; nothing chosen yet: "Choose a variant" first.
   */
  pick(screenId: string, layerId: string, current: string): (VariantOpt & { disabled?: boolean })[] {
    const opts = this.ofLayer(screenId, layerId);
    const head = current ? [] : [{ value: "", label: "Choose a variant", disabled: true }];
    if (opts.length === 0) return current ? [{ value: current, label: current }] : head;
    if (current && !opts.some((o) => o.value === current)) return [...opts, { value: current, label: `Missing (${current})` }];
    return [...head, ...opts];
  }

  /** Forget everything (an editor opened: screens and templates may have changed meanwhile). */
  reset(): void {
    this.#templates = {};
    this.#screens = {};
  }

  #loadTemplate(key: string, id: string) {
    if (key in this.#templates) return;
    this.#templates = { ...this.#templates, [key]: null };
    getTemplate(id).then(
      (doc) => {
        const loaded: Loaded = {
          defaultVariant: doc.defaultVariant,
          variants: doc.variants.map((v) => ({ id: v.id, name: v.name })),
        };
        this.#templates = { ...this.#templates, [key]: loaded };
      },
      () => {},
    );
  }

  #loadScreen(id: string) {
    if (id in this.#screens) return;
    this.#screens = { ...this.#screens, [id]: null };
    getScreen(id).then(
      (st) => {
        this.#screens = { ...this.#screens, [id]: Object.fromEntries(st.doc.layers.flatMap((l) => (l.kind !== "media" ? [[l.id, l.templateId]] : []))) };
      },
      () => {},
    );
  }
}

export const variantCache = new VariantCache();
