// Video posters for the media library: the first frame of a file, drawn into the tile's canvas.
//
// A tile asks only when it scrolls into view (`posterAttachment`, one IntersectionObserver for
// all tiles). One hidden <video> at a time decodes a frame (preload metadata, seek to 0.1 s or the
// middle of very short files), the frame is kept as a small ImageBitmap (cached by file + mtime,
// at most 64) and the video element is released at once (src removed), so no decoder stays alive.
// Cross-origin frames taint the canvas, which only blocks reading pixels: drawing them is fine.

const W = 192;
const H = 108;
const CACHE_MAX = 64;
const TIMEOUT_MS = 8000;

const cache = new Map<string, ImageBitmap | null>();
const queue: { key: string; url: string; resolve: (b: ImageBitmap | null) => void }[] = [];
let running = false;

async function grab(url: string): Promise<ImageBitmap | null> {
  const v = document.createElement("video");
  v.muted = true;
  v.preload = "metadata";
  v.playsInline = true;
  try {
    const frame = await new Promise<boolean>((resolve) => {
      const timer = setTimeout(() => resolve(false), TIMEOUT_MS);
      const done = (ok: boolean) => {
        clearTimeout(timer);
        resolve(ok);
      };
      v.onloadedmetadata = () => {
        const d = Number.isFinite(v.duration) ? v.duration : 0;
        v.currentTime = d > 0 && d < 0.4 ? d / 2 : 0.1;
      };
      v.onseeked = () => done(v.videoWidth > 0);
      v.onerror = () => done(false);
      v.src = url;
    });
    if (!frame) return null;
    const scale = Math.min(W / v.videoWidth, H / v.videoHeight, 1);
    return await createImageBitmap(v, {
      resizeWidth: Math.max(1, Math.round(v.videoWidth * scale)),
      resizeHeight: Math.max(1, Math.round(v.videoHeight * scale)),
      resizeQuality: "medium",
    });
  } catch {
    return null;
  } finally {
    v.onloadedmetadata = v.onseeked = v.onerror = null;
    v.removeAttribute("src");
    v.load();
  }
}

async function pump() {
  if (running) return;
  running = true;
  try {
    while (queue.length > 0) {
      const job = queue.shift()!;
      const hit = cache.get(job.key);
      if (hit !== undefined) {
        job.resolve(hit);
        continue;
      }
      const bmp = await grab(job.url);
      cache.set(job.key, bmp);
      while (cache.size > CACHE_MAX) {
        const first = cache.keys().next().value as string;
        cache.get(first)?.close();
        cache.delete(first);
      }
      job.resolve(bmp);
    }
  } finally {
    running = false;
  }
}

/** The poster of a video (null when it can't be decoded here). `key` changes with the file. */
export function poster(key: string, url: string): Promise<ImageBitmap | null> {
  const hit = cache.get(key);
  if (hit !== undefined) return Promise.resolve(hit);
  return new Promise((resolve) => {
    queue.push({ key, url, resolve });
    void pump();
  });
}

/** Draws a bitmap centered into a canvas (letterboxed). */
export function drawPoster(canvas: HTMLCanvasElement, bmp: ImageBitmap) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  canvas.width = W;
  canvas.height = H;
  ctx.clearRect(0, 0, W, H);
  const s = Math.min(W / bmp.width, H / bmp.height);
  const w = bmp.width * s;
  const h = bmp.height * s;
  ctx.drawImage(bmp, (W - w) / 2, (H - h) / 2, w, h);
}

let observer: IntersectionObserver | null = null;
const pending = new Map<Element, () => void>();

/**
 * Svelte attachment for a tile's canvas: draws the poster once the canvas is in view. Returns the
 * cleanup (stops observing).
 */
export function posterAttachment(key: string, url: string | null) {
  return (canvas: HTMLCanvasElement) => {
    if (!url) return;
    let alive = true;
    const start = () => {
      void poster(key, url).then((bmp) => {
        if (alive && bmp) {
          drawPoster(canvas, bmp);
          canvas.dataset.poster = "";
        }
      });
    };
    if (typeof IntersectionObserver === "undefined") {
      start();
      return () => {
        alive = false;
      };
    }
    observer ??= new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        const go = pending.get(e.target);
        pending.delete(e.target);
        observer?.unobserve(e.target);
        go?.();
      }
    });
    pending.set(canvas, start);
    observer.observe(canvas);
    return () => {
      alive = false;
      pending.delete(canvas);
      observer?.unobserve(canvas);
    };
  };
}
