// Flow names: the default "<Base> N" and the name check. Shared by the linear editor (./edit.ts
// re-exports them), the node editor and the /flows list.

import { FLOW_NAME_MAX } from "$lib/types";

/** "<Base> N": the first number not taken. */
export function defaultName(base: string, names: readonly string[]): string {
  const taken = new Set(names.map((n) => n.trim().toLowerCase()));
  let n = 1;
  while (taken.has(`${base.toLowerCase()} ${n}`)) n++;
  return `${base} ${n}`;
}

/** A flow-level error (name, step count), or null. */
export function nameError(name: string): string | null {
  const n = name.trim();
  if (n.length === 0) return "Give the flow a name.";
  if (n.length > FLOW_NAME_MAX) return `Names are at most ${FLOW_NAME_MAX} characters.`;
  return null;
}
