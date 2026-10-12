// Control's template variants (part of LayerBody's lazy chunk): switching a layer's variant with the transition the operator
// chose (remembered for the app session), and the palette commands "Switch <layer> to <variant>"
// for the selected screen. The variant names come from the shared cache (one get_template per
// template, again when the template's updatedAt changes).

import { toAppError } from "$lib/api";
import { setLayerVariant } from "$lib/api/screens";
import { variantCache, watchVariantCommands, type VariantLayer } from "$lib/components/screens/variants.svelte";
import { live } from "$lib/stores/live.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import type { VariantTransition } from "$lib/types";
import type { ControlCtl } from "./control.svelte";

/** The transition an on-air switch uses; the operator's last choice for this app session. */
export const session = $state<{ transition: VariantTransition }>({ transition: "crossfade" });

/**
 * Shows `variant` on a layer (optimistic: the select follows at once; rolled back on failure).
 * Off air it just changes; on air it follows Live edit (staged as "Changes on next take" under
 * onTake, else swapped with the session's transition).
 */
export async function switchVariant(screenId: string, layerId: string, variant: string): Promise<void> {
  const layer = live.screens.get(screenId)?.layers.find((l) => l.id === layerId);
  if (!layer || layer.variant === variant) return;
  const next = { ...layer, variant };
  live.putLayer(screenId, next);
  try {
    live.put((await setLayerVariant(screenId, layerId, variant, session.transition)).live);
  } catch (e) {
    const now = live.screens.get(screenId)?.layers.find((l) => l.id === layerId);
    if (now === next) live.putLayer(screenId, layer);
    toasts.push({ kind: "error", message: `Couldn't switch ${layer.name}: ${toAppError(e).message}` });
  }
}

/** The selected screen's layers with several variants, as the palette lists them. */
function variantLayers(ctl: ControlCtl, screenId: string): VariantLayer[] {
  const out: VariantLayer[] = [];
  for (const l of live.screens.get(screenId)?.layers ?? []) {
    const t = ctl.templateOf(l.id);
    if (!t || t.variantCount < 2) continue;
    const v = variantCache.template(t.id, t.updatedAt);
    if (!v) continue;
    out.push({ screenId, layerId: l.id, name: l.name, current: l.variant || v.defaultVariant, variants: v.variants });
  }
  return out;
}

/**
 * Registers "Switch <layer> to <variant>" for the screen's multi-variant layers and keeps them
 * current (names, the variant shown); returns the function that removes them.
 */
export function registerVariantCommands(ctl: ControlCtl, screenId: string): () => void {
  return watchVariantCommands(
    () => variantLayers(ctl, screenId),
    { section: "live", keyScope: "control" },
    (l, v) => switchVariant(l.screenId, l.layerId, v),
  );
}
