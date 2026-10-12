// Tiny RGBA PNG writer for the UI harness's sponsor logos and project images (no dependencies):
// a rounded tile in one colour with a geometric mark and "wordmark" bars in a second colour,
// 2×2 supersampled so edges are smooth.

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { deflateSync } from "node:zlib";

type Rgb = [number, number, number];

export interface Logo {
  file: string;
  bg: Rgb;
  fg: Rgb;
  mark: "circle" | "triangle" | "diamond" | "bars" | "ring" | "wave";
  /** Wordmark bar widths as a fraction of the free width (1–3 bars). */
  words?: number[];
  width?: number;
  height?: number;
}

const CRC = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc(b: Buffer): number {
  let c = 0xffffffff;
  for (const x of b) c = CRC[(c ^ x) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type: string, data: Buffer): Buffer {
  const t = Buffer.from(type, "ascii");
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const c = Buffer.alloc(4);
  c.writeUInt32BE(crc(Buffer.concat([t, data])));
  return Buffer.concat([len, t, data, c]);
}

/** Encodes RGBA pixels (row-major, 4 bytes each) as a PNG. */
export function encodePng(w: number, h: number, rgba: Uint8Array): Buffer {
  const raw = Buffer.alloc((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 4 + 1)] = 0;
    raw.set(rgba.subarray(y * w * 4, (y + 1) * w * 4), y * (w * 4 + 1) + 1);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr.set([8, 6, 0, 0, 0], 8);
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/** Writes the logo tile into `dir`; returns its absolute path. */
export function logoPng(dir: string, logo: Logo): string {
  const w = logo.width ?? 240;
  const h = logo.height ?? 120;
  const r = h * 0.16;
  const cy = h / 2;
  const mx = h * 0.5; // mark centre x
  const ms = h * 0.26; // mark half size
  const words = logo.words ?? [0.9, 0.55];
  const wx0 = h * 0.92;
  const free = w - wx0 - h * 0.18;
  const barH = h * 0.12;
  const gap = h * 0.09;
  const total = words.length * barH + (words.length - 1) * gap;

  const inTile = (x: number, y: number) => {
    const dx = Math.max(r - x, 0, x - (w - r));
    const dy = Math.max(r - y, 0, y - (h - r));
    return dx * dx + dy * dy <= r * r;
  };
  const inMark = (x: number, y: number): boolean => {
    const dx = x - mx;
    const dy = y - cy;
    switch (logo.mark) {
      case "circle":
        return dx * dx + dy * dy <= ms * ms;
      case "ring": {
        const d = Math.hypot(dx, dy);
        return d <= ms && d >= ms * 0.58;
      }
      case "diamond":
        return Math.abs(dx) + Math.abs(dy) <= ms * 1.15;
      case "triangle":
        return dy <= ms * 0.85 && dy >= -ms && Math.abs(dx) <= ((dy + ms) / (ms * 1.85)) * ms * 1.1;
      case "bars": {
        if (Math.abs(dy) > ms || Math.abs(dx) > ms) return false;
        const col = Math.floor(((dx + ms) / (2 * ms)) * 3);
        const local = ((dx + ms) / (2 * ms)) * 3 - col;
        const top = [-0.2, -1, -0.55][col] ?? 0;
        return local < 0.7 && dy >= top * ms;
      }
      case "wave":
        return Math.abs(dx) <= ms && Math.abs(dy - Math.sin((dx / ms) * Math.PI) * ms * 0.45) <= ms * 0.22;
    }
  };
  const inWords = (x: number, y: number) => {
    const y0 = cy - total / 2;
    for (let i = 0; i < words.length; i++) {
      const top = y0 + i * (barH + gap);
      if (y >= top && y <= top + barH && x >= wx0 && x <= wx0 + free * words[i]) return true;
    }
    return false;
  };

  const px = new Uint8Array(w * h * 4);
  const S = [0.25, 0.75];
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let a = 0;
      let f = 0;
      for (const sy of S) {
        for (const sx of S) {
          const X = x + sx;
          const Y = y + sy;
          if (!inTile(X, Y)) continue;
          a++;
          if (inMark(X, Y) || inWords(X, Y)) f++;
        }
      }
      const i = (y * w + x) * 4;
      if (a === 0) continue;
      const t = f / a;
      for (let k = 0; k < 3; k++) px[i + k] = Math.round(logo.bg[k] * (1 - t) + logo.fg[k] * t);
      px[i + 3] = Math.round((a / 4) * 255);
    }
  }
  const path = join(dir, logo.file);
  writeFileSync(path, encodePng(w, h, px));
  return path;
}
