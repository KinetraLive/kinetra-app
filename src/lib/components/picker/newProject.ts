// Path helpers for the New project form only. Kept apart from ./format so they load with the form
// (lazy) instead of with the picker's first paint (budget: 50 KB gz).

/** Approximates `core::project::slug` (src-tauri) for the "creates <location>\<slug>" hint. */
export function slugify(name: string): string {
  const s = name
    .trim()
    .toLowerCase()
    .replace(/[\s-]+/g, "-")
    .replace(/[^\p{L}\p{N}_.-]/gu, "")
    .replace(/-{2,}/g, "-")
    .slice(0, 64)
    .replace(/^[-. ]+|[-. ]+$/g, "");
  return s || "project";
}

/** Joins a folder and a child name with the separator the folder already uses. */
export function joinPath(parent: string, child: string): string {
  const sep = parent.includes("\\") ? "\\" : "/";
  return parent.replace(/[\\/]+$/, "") + sep + child;
}
