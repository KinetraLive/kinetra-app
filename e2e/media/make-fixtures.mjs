// Generates the e2e test clips in the installed Edge (no ffmpeg needed): small canvas animations
// recorded with MediaRecorder. Run: `node e2e/media/make-fixtures.mjs` (writes e2e/media/*).
//
// Every frame carries its frame number (30 fps) as a 10-bit code: blocks of 40×40 px at
// x = 20 + 48·i, y = 20 (bit i, least significant first), white = 1, black = 0, on a mid-grey
// stripe; plus the number as text and a moving bar. Tests read the code back from a canvas
// (`readFrame` in e2e/media.spec.ts), which survives lossy compression.
//
// MediaRecorder writes live WebM without a Duration (and fragmented MP4): the WebM's Segment Info
// gets a Duration element patched in here, so players and the probe know the length and can seek.
import { chromium } from "@playwright/test";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));
const FPS = 30;

/** name, mime, seconds, alpha */
const CLIPS = [
  ["clip-vp8.webm", "video/webm;codecs=vp8", 3, false],
  ["clip-vp9.webm", "video/webm;codecs=vp9", 3, false],
  ["clip-vp9-alpha.webm", "video/webm;codecs=vp9", 2, true],
  ["clip-h264.mp4", "video/mp4;codecs=avc1.42E01E", 3, false],
];

const browser = await chromium.launch({ channel: "msedge" });
const page = await browser.newPage();
await page.goto("about:blank");

for (const [name, mime, secs, alpha] of CLIPS) {
  const ok = await page.evaluate((m) => MediaRecorder.isTypeSupported(m), mime);
  if (!ok) {
    console.log(`${name}: ${mime} not supported by this Edge's MediaRecorder, skipped`);
    continue;
  }
  const b64 = await page.evaluate(
    async ({ mime, secs, alpha, FPS }) => {
      const W = 480;
      const H = 270;
      const c = document.createElement("canvas");
      c.width = W;
      c.height = H;
      const g = c.getContext("2d", { alpha });
      const draw = (n) => {
        g.clearRect(0, 0, W, H);
        if (!alpha) {
          g.fillStyle = `hsl(${(n * 4) % 360} 60% 35%)`;
          g.fillRect(0, 0, W, H);
        } else {
          // Transparent page, an opaque coloured card in the middle.
          g.fillStyle = "#e0402a";
          g.fillRect(120, 90, 240, 150);
        }
        g.fillStyle = "#808080";
        g.fillRect(10, 10, 10 * 48 + 12, 60);
        for (let i = 0; i < 10; i++) {
          g.fillStyle = (n >> i) & 1 ? "#ffffff" : "#000000";
          g.fillRect(20 + 48 * i, 20, 40, 40);
        }
        g.fillStyle = "#ffffff";
        g.font = "bold 64px sans-serif";
        g.fillText(String(n), 24, 160);
        g.fillStyle = "#ffd400";
        g.fillRect(((n * 8) % (W - 20)) | 0, H - 30, 20, 20);
      };
      const stream = c.captureStream(0);
      const track = stream.getVideoTracks()[0];
      const rec = new MediaRecorder(stream, { mimeType: mime, videoBitsPerSecond: 400_000 });
      const parts = [];
      rec.ondataavailable = (e) => e.data.size && parts.push(e.data);
      const stopped = new Promise((r) => (rec.onstop = r));
      draw(0);
      rec.start();
      const t0 = performance.now();
      const total = secs * FPS;
      // Frame n is pushed at t0 + n/FPS (wall clock: MediaRecorder stamps frames on arrival).
      for (let n = 0; n < total; n++) {
        const due = t0 + (n * 1000) / FPS;
        while (performance.now() < due - 2) await new Promise((r) => setTimeout(r, Math.max(0, due - performance.now() - 2)));
        draw(n);
        track.requestFrame();
      }
      await new Promise((r) => setTimeout(r, 1000 / FPS));
      rec.stop();
      await stopped;
      const buf = new Uint8Array(await new Blob(parts).arrayBuffer());
      let s = "";
      for (let i = 0; i < buf.length; i += 0x8000) s += String.fromCharCode(...buf.subarray(i, i + 0x8000));
      return btoa(s);
    },
    { mime, secs, alpha, FPS },
  );
  let bytes = Buffer.from(b64, "base64");
  if (name.endsWith(".webm")) bytes = patchWebmDuration(bytes, secs * 1000);
  writeFileSync(join(dir, name), bytes);
  console.log(`${name}: ${bytes.length} bytes`);
}
await browser.close();

/** Reads an EBML variable-size integer at `o`: [value, length]. */
function vint(b, o) {
  const first = b[o];
  let len = 1;
  while (len <= 8 && !(first & (0x80 >> (len - 1)))) len++;
  let v = first & (0xff >> len);
  for (let i = 1; i < len; i++) v = v * 256 + b[o + i];
  return [v, len];
}

/** Element id at `o` (kept with its marker bits): [id, length]. */
function eid(b, o) {
  const [, len] = vint(b, o);
  let id = 0;
  for (let i = 0; i < len; i++) id = id * 256 + b[o + i];
  return [id, len];
}

/** An 8-byte EBML size. */
function size8(n) {
  const s = Buffer.alloc(8);
  s[0] = 0x01;
  s.writeUIntBE(n, 2, 6);
  return s;
}

/**
 * Adds (or replaces) Segment › Info › Duration (float64, in TimecodeScale units: ms with the
 * default 1 000 000 ns) so the file has a known length. The Info element is rewritten with an
 * 8-byte size; the Segment's size is MediaRecorder's "unknown", so nothing else moves.
 */
function patchWebmDuration(b, ms) {
  let o = 0;
  const [hid, hl] = eid(b, o);
  if (hid !== 0x1a45dfa3) throw new Error("not EBML");
  const [hs, hsl] = vint(b, o + hl);
  o += hl + hsl + hs; // Segment
  const [sid, sl] = eid(b, o);
  if (sid !== 0x18538067) throw new Error("no Segment");
  const [, ssl] = vint(b, o + sl);
  o += sl + ssl;
  for (;;) {
    const [id, il] = eid(b, o);
    const [sz, szl] = vint(b, o + il);
    if (id === 0x1549a966) {
      const start = o + il + szl;
      const kids = [];
      for (let p = start; p < start + sz; ) {
        const [kid, kl] = eid(b, p);
        const [ks, ksl] = vint(b, p + kl);
        if (kid !== 0x4489) kids.push(b.subarray(p, p + kl + ksl + ks));
        p += kl + ksl + ks;
      }
      const dur = Buffer.alloc(11);
      dur[0] = 0x44;
      dur[1] = 0x89;
      dur[2] = 0x88;
      dur.writeDoubleBE(ms, 3);
      const body = Buffer.concat([...kids, dur]);
      return Buffer.concat([b.subarray(0, o), Buffer.from([0x15, 0x49, 0xa9, 0x66]), size8(body.length), body, b.subarray(start + sz)]);
    }
    o += il + szl + sz;
    if (o >= b.length) throw new Error("no Info");
  }
}
