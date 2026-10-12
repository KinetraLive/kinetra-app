// The layout editor's lazy overlays chunk: rulers and guides, the view overlays (grid, safe areas,
// centre cross, crop), smart guides and their maths. Canvas requests it when its module loads, so
// the /screens/[id] route's initial JS stays small; until it arrives the canvas works with plain
// snapping and no overlays.

export { default as Rulers } from "./Rulers.svelte";
export { default as SmartGuides } from "./SmartGuides.svelte";
export { default as ViewOverlays } from "./ViewOverlays.svelte";
export { smartContext, smartGuides, snapSpacing } from "./smart";
