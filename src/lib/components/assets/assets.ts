// Pure helpers for image values (`/assets/<name>`) and the asset picker. No Svelte, no I/O.
// Small and dependency-free: image fields, image cells and Control import it statically.

import type { AssetInfo, ServerStatus } from "$lib/types";

const PREFIX = "/assets/";

/** True for a project image value (`/assets/<name>`). */
export const isAssetValue = (v: string) => v.startsWith(PREFIX) && v.length > PREFIX.length;

/** The file name of an `/assets/<name>` value (decoded), else the value itself. */
export function assetName(v: string): string {
  if (!isAssetValue(v)) return v;
  const raw = v.slice(PREFIX.length);
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

/**
 * The URL the app UI shows a thumbnail from: the running output server's base URL + the value.
 * Null when the server isn't running or the value is not a project image.
 */
export function thumbUrl(status: ServerStatus | null, v: string): string | null {
  if (status?.state !== "running" || !isAssetValue(v)) return null;
  return `${status.url.replace(/\/+$/, "")}${v}`;
}

/** True for a web image link (http/https): shown on air from the project's copy. */
export const isWebImage = (v: string) => /^https?:\/\/\S+$/i.test(v.trim());

/** The thumbnail of a web image link: its copy through the running output server (`/web-image`,
 *  which downloads it when a layer holds the link). Null when the server isn't running. */
export function webThumbUrl(status: ServerStatus | null, v: string, attempt = 0): string | null {
  if (status?.state !== "running" || !isWebImage(v)) return null;
  return `${status.url.replace(/\/+$/, "")}/web-image?u=${encodeURIComponent(v.trim())}${attempt ? `&try=${attempt}` : ""}`;
}

/** "12 KB", "1.4 MB", "2.1 GB" (binary units, one decimal from MB). */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** Assets whose name contains `query` (case-insensitive), sorted by name. */
export function filterAssets(list: readonly AssetInfo[], query: string): AssetInfo[] {
  const q = query.trim().toLowerCase();
  return list
    .filter((a) => q === "" || a.name.toLowerCase().includes(q))
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}

/** Why a new asset name can't be used, or null. The extension must stay the same. `noun`: what
 * another file with that name is called in the message. */
export function renameIssue(name: string, current: string, taken: readonly string[], noun = "image"): string | null {
  const n = name.trim();
  if (n === "") return "Enter a name.";
  if (n.length > 128) return "Up to 128 characters.";
  if (/[\/:*?"<>|]/.test(n) || n.startsWith(".")) return "Use letters, digits, spaces, - _ or .";
  const ext = (s: string) => s.slice(s.lastIndexOf(".") + 1).toLowerCase();
  if (!n.includes(".") || ext(n) !== ext(current)) return `Keep the .${ext(current)} extension.`;
  if (n !== current && taken.some((t) => t.toLowerCase() === n.toLowerCase())) return `Another ${noun} has this name.`;
  return null;
}
