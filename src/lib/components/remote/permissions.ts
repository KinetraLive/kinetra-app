// Collaborators' permissions (protocol 2 §2; core::remote::perms is the app's check): the codes
// in form order with their labels, the three presets that fill the checkboxes, and the short
// summary the people list shows. Unknown codes (later additions) are ignored everywhere.

export type Permission =
  | "take_clear"
  | "edit_fields"
  | "edit_live_fields"
  | "run_cues"
  | "studio"
  | "timers"
  | "variants"
  | "rows"
  | "button_panel"
  | "watch_preview";

/** Every permission, in the order the form lists them. */
export const PERMISSIONS: readonly { code: Permission; label: string }[] = [
  { code: "take_clear", label: "Take and clear graphics" },
  { code: "edit_fields", label: "Edit graphics that aren't on air" },
  { code: "edit_live_fields", label: "Edit graphics on air" },
  { code: "run_cues", label: "Run cues" },
  { code: "studio", label: "Studio: cue and take" },
  { code: "timers", label: "Start, pause and reset timers" },
  { code: "variants", label: "Change looks" },
  { code: "rows", label: "Pick data rows" },
  { code: "button_panel", label: "Press panel buttons" },
  { code: "watch_preview", label: "Watch the live picture" },
];

const KNOWN = new Set<string>(PERMISSIONS.map((p) => p.code));

export type Preset = "full" | "data" | "watch";

export const PRESETS: readonly { id: Preset; label: string; codes: readonly Permission[] }[] = [
  { id: "full", label: "Full operator", codes: PERMISSIONS.map((p) => p.code) },
  { id: "data", label: "Data editor", codes: ["edit_fields", "rows", "watch_preview"] },
  { id: "watch", label: "Watch only", codes: ["watch_preview"] },
];

/** Known codes only, in form order, without duplicates; editing on air brings editing off air. */
export function normalize(codes: readonly string[]): Permission[] {
  const set = new Set(codes.filter((c) => KNOWN.has(c)));
  if (set.has("edit_live_fields")) set.add("edit_fields");
  return PERMISSIONS.map((p) => p.code).filter((c) => set.has(c));
}

/** The preset these codes are exactly, else null ("Custom"). */
export function presetOf(codes: readonly string[]): Preset | null {
  const key = normalize(codes).join(",");
  return PRESETS.find((p) => normalize(p.codes).join(",") === key)?.id ?? null;
}

/** "Full operator", "Data editor", "Watch only", "Custom" or "No permissions". A protocol 1
 *  crew has a role instead: shown by what it allowed. */
export function summaryOf(p: { permissions?: readonly string[] | null; role?: string }): string {
  if (p.permissions) {
    const preset = presetOf(p.permissions);
    if (preset) return PRESETS.find((x) => x.id === preset)!.label;
    return normalize(p.permissions).length === 0 ? "No permissions" : "Custom";
  }
  if (p.role === "operator") return "Full operator";
  if (p.role === "data_editor") return "Data editor";
  return "Watch only";
}
