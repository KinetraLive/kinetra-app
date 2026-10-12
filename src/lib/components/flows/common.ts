// Small flow helpers the /flows list needs at first paint (new ids, "3 min ago"), apart from
// linear.ts so the list's route chunk doesn't carry the summary code; linear.ts re-exports them.

const ID_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

/** nanoid-style id: 10 random chars from [A-Za-z0-9_-]. */
export function newNodeId(): string {
  let id = "";
  for (const b of crypto.getRandomValues(new Uint8Array(10))) id += ID_ALPHABET[b & 63];
  return id;
}

/**
 * "12 s ago", "3 min ago", "2 h ago", "4 d ago" (formatted on render; no timers). The same as
 * flows.svelte's relTime: not imported from there, so that module stays inside the Control route's
 * chunk instead of being split out for these lazy chunks.
 */
export function relTime(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return `${s} s ago`;
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}
