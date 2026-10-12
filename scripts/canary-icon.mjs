// The Kinetra Canary icon: the stable icon with a "CANARY" band across the bottom, drawn in the
// installed Edge (no image library), then every size made by `tauri icon`.
//
//   node scripts/canary-icon.mjs [source.png]     (default src-tauri/icons/kinetra-app-icon-1024.png)
//
// Writes src-tauri/icons/canary/ (used by src-tauri/tauri.canary.conf.json). Run it again
// whenever the stable icon changes.

import { execSync } from "node:child_process";
import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = resolve(process.argv[2] ?? join(root, "src-tauri", "icons", "kinetra-app-icon-1024.png"));
const outDir = join(root, "src-tauri", "icons", "canary");
const master = join(outDir, "canary-1024.png");

const browser = await chromium.launch({ channel: "msedge" });
try {
  const page = await browser.newPage();
  const dataUrl = `data:image/png;base64,${readFileSync(source).toString("base64")}`;
  const png = await page.evaluate(async (src) => {
    const img = new Image();
    img.src = src;
    await img.decode();
    const size = 1024;
    const c = document.createElement("canvas");
    c.width = size;
    c.height = size;
    const g = c.getContext("2d");
    g.imageSmoothingQuality = "high";
    g.drawImage(img, 0, 0, size, size);
    // The band: canary yellow, a quarter of the height, inside the icon's lower edge.
    const h = Math.round(size * 0.24);
    const y = size - h - Math.round(size * 0.06);
    // Clipped to the icon's own shape (source-atop), so rounded corners stay rounded.
    g.globalCompositeOperation = "source-atop";
    g.fillStyle = "#f5c400";
    g.fillRect(0, y, size, h);
    g.fillStyle = "#111111";
    g.font = `700 ${Math.round(h * 0.62)}px "Segoe UI", system-ui, sans-serif`;
    g.textAlign = "center";
    g.textBaseline = "middle";
    g.fillText("CANARY", size / 2, y + h / 2 + Math.round(h * 0.03));
    return c.toDataURL("image/png").split(",")[1];
  }, dataUrl);
  mkdirSync(outDir, { recursive: true });
  writeFileSync(master, Buffer.from(png, "base64"));
} finally {
  await browser.close();
}

execSync(`pnpm tauri icon "${master}" -o "${outDir}"`, { cwd: root, stdio: "inherit" });
// Keep what the bundle config lists (desktop only).
const keep = new Set(["canary-1024.png", "32x32.png", "128x128.png", "128x128@2x.png", "icon.icns", "icon.ico"]);
for (const name of readdirSync(outDir)) {
  if (!keep.has(name)) rmSync(join(outDir, name), { recursive: true, force: true });
}
console.log(`canary icons in ${outDir}`);
