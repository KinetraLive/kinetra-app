// Screen id and WebSocket URL from the page location.

/** Screen id from a path like /screen/<id>, or "". */
export function screenId(path: string): string {
  const m = /\/screen\/([^/?#]+)/.exec(path);
  try {
    return m ? decodeURIComponent(m[1]) : "";
  } catch {
    return "";
  }
}

/** The preview bus page: /screen/<id>/preview (optional trailing slash). */
export const isPreview = (path: string) => /\/screen\/[^/?#]+\/preview\/?$/.test(path);

/** Same host as the page; wss: when the page is https:. The preview bus adds `&bus=preview`. */
export function wsUrl(loc: { protocol: string; host: string }, id: string, preview = false): string {
  return `${loc.protocol === "https:" ? "wss:" : "ws:"}//${loc.host}/ws?screen=${encodeURIComponent(id)}${preview ? "&bus=preview" : ""}`;
}
