// Media playback UI shots (part 3): the media library (with playback issues and Convert…), an
// import in progress, the layout editor's media Inspector, Control's media rows (transport,
// elapsed / remaining, readiness) and Preferences › Outputs with the audio device per output.
// `pnpm shots:ui -g "media"`; PNGs in review/kinetra-m7/ (gitignored).

import { mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { SCREEN_DOCS, mirrorScreens, open, screensScenario } from "./harness";
import type { MockScenario } from "./mock-tauri";
import type { AssetInfo, MediaInfo, OutputConfig, ScreenDoc } from "../../src/lib/types";

const OUT = join(process.cwd(), "review", "kinetra-m7");
mkdirSync(OUT, { recursive: true });

async function shot(page: Page, name: string) {
  await page.waitForTimeout(600);
  await page.screenshot({ path: join(OUT, `${name}.png`) });
}

const MEDIA_SCREEN = "s9media01x";

const info = (over: Partial<MediaInfo>): MediaInfo => ({
  kind: "video",
  container: "mp4",
  durationMs: 12_480,
  width: 1920,
  height: 1080,
  fps: 25,
  videoCodec: "h264 High 4.1",
  audioCodec: "aac",
  alpha: false,
  fastStart: true,
  issues: [],
  ...over,
});

const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString();

/** The library: real clips imported into the fixture (posters from their first frame). */
async function libraryAssets(server: { cmd(line: string): Promise<string> }): Promise<AssetInfo[]> {
  const imp = async (file: string) => {
    try {
      return await server.cmd(`importasset ${resolve("e2e/media", file)}`);
    } catch {
      // The fixture may not take video yet: the tile then shows its icon.
      return `/assets/${file}`;
    }
  };
  const opener = await imp("clip-h264.mp4");
  const sting = await imp("clip-vp9-alpha.webm");
  const loop = await imp("clip-vp9.webm");
  const hevcCmd = 'ffmpeg -i "interview-hevc.mp4" -c:v libx264 -crf 18 -pix_fmt yuv420p -c:a aac -movflags +faststart "interview-hevc-h264.mp4"';
  return [
    { name: decodeURIComponent(opener.slice(8)), url: opener, bytes: 76_494, modifiedAt: ago(30), usedBy: [MEDIA_SCREEN], media: info({}) },
    {
      name: decodeURIComponent(sting.slice(8)),
      url: sting,
      bytes: 54_276,
      modifiedAt: ago(25),
      usedBy: [MEDIA_SCREEN],
      media: info({
        container: "webm",
        durationMs: 4_000,
        videoCodec: "vp9",
        audioCodec: null,
        alpha: true,
        fastStart: null,
        issues: [
          {
            code: "noAlpha",
            severity: "error",
            platforms: ["macos"],
            message: "Shows on black on Mac.",
            convert: {
              label: "Convert to HEVC with alpha (.mov) for Mac",
              command: 'ffmpeg -i "clip-vp9-alpha.webm" -c:v hevc_videotoolbox -alpha_quality 0.75 -vtag hvc1 "clip-vp9-alpha.mov"',
            },
          },
          { code: "unsupported", severity: "warning", platforms: ["macos"], message: "Needs macOS 11.3 or later.", convert: null },
        ],
      }),
    },
    { name: decodeURIComponent(loop.slice(8)), url: loop, bytes: 91_025, modifiedAt: ago(20), usedBy: [], media: info({ container: "webm", videoCodec: "vp9", audioCodec: "opus", fastStart: null, durationMs: 8_000 }) },
    {
      name: "interview-hevc.mp4",
      url: "/assets/interview-hevc.mp4",
      bytes: 1_843_200_000,
      modifiedAt: ago(10),
      usedBy: [],
      media: info({
        durationMs: 1_520_000,
        width: 3840,
        height: 2160,
        fps: 29.97,
        videoCodec: "hevc Main 5.1",
        fastStart: false,
        issues: [
          { code: "unsupported", severity: "error", platforms: ["obs"], message: "Won't play in OBS: HEVC.", convert: { label: "Convert to H.264", command: hevcCmd } },
          { code: "needsExtension", severity: "warning", platforms: ["windows"], message: "Needs the HEVC extension on Windows.", convert: null },
          { code: "slowStart", severity: "warning", platforms: ["obs", "vmix"], message: "Starts slowly over the network.", convert: { label: "Re-save with fast start", command: 'ffmpeg -i "interview-hevc.mp4" -c copy -movflags +faststart "interview-hevc-faststart.mp4"' } },
        ],
      }),
    },
    { name: "walk-in-bed.mp3", url: "/assets/walk-in-bed.mp3", bytes: 7_340_032, modifiedAt: ago(8), usedBy: [], media: info({ kind: "audio", container: "mp3", width: null, height: null, fps: null, videoCodec: null, audioCodec: "mp3", fastStart: null, durationMs: 183_000 }) },
  ];
}

const mediaDoc = (assets: AssetInfo[]): ScreenDoc => ({
  id: MEDIA_SCREEN,
  name: "Show opener",
  width: 1920,
  height: 1080,
  background: "transparent",
  layers: [
    { id: "lbed000001", name: "Walk-in bed", kind: "media", rect: { x: 0, y: 0, w: 1920, h: 1080 }, media: { asset: assets[4].url, fit: "fit", inMs: 0, outMs: null, volume: 70, muted: false, endAction: "loop", autoplay: true } },
    { id: "lopener001", name: "Opener", kind: "media", rect: { x: 0, y: 0, w: 1920, h: 1080 }, media: { asset: assets[0].url, fit: "fill", inMs: 1_200, outMs: 11_000, volume: 100, muted: false, endAction: "hold", autoplay: true } },
    { id: "lsting0001", name: "Sting", kind: "media", rect: { x: 1440, y: 60, w: 420, h: 236 }, media: { asset: assets[1].url, fit: "fit", inMs: 0, outMs: null, volume: 100, muted: true, endAction: "clear", autoplay: true } },
  ],
  updatedAt: ago(5),
});

function scenario(theme: "dark" | "light", assets: AssetInfo[], over: Partial<MockScenario> = {}): MockScenario {
  const base = screensScenario(theme);
  const now = Date.now();
  return {
    ...base,
    screenDocs: [...SCREEN_DOCS, mediaDoc(assets)],
    assets,
    onAirLayers: [...(base.onAirLayers ?? []), `${MEDIA_SCREEN}/lopener001`, `${MEDIA_SCREEN}/lbed000001`],
    mediaLive: {
      [`${MEDIA_SCREEN}/lopener001`]: { playback: { state: "playing", at: now - 4_300, posMs: 1_200 }, readiness: { ready: 3, total: 3, error: null }, durationMs: 12_480 },
      [`${MEDIA_SCREEN}/lbed000001`]: { playback: { state: "paused", at: now, posMs: 42_000 }, readiness: { ready: 3, total: 3, error: null }, durationMs: 183_000 },
      [`${MEDIA_SCREEN}/lsting0001`]: { playback: { state: "stopped", at: 0, posMs: 0 }, readiness: { ready: 2, total: 3, error: null }, durationMs: 4_000 },
    },
    ...over,
  };
}

const AUDIO_OUTPUTS: OutputConfig[] = [
  {
    id: "out1fill00",
    kind: "display",
    name: "Program fill",
    enabled: true,
    display: { id: "disp-fill", name: "ASUS PA248QV", edidKey: "AUS-24A1-7", width: 1920, height: 1080 },
    screenId: MEDIA_SCREEN,
    mode: "fill",
    background: { kind: "black" },
    keyOf: null,
    alwaysOnTop: true,
    audio: { enabled: true, device: "Speakers (Focusrite USB Audio)" },
  },
  {
    id: "out2key000",
    kind: "display",
    name: "Program key",
    enabled: true,
    display: { id: "disp-key", name: "ASUS PA248QV", edidKey: "AUS-24A1-9", width: 1920, height: 1080 },
    screenId: null,
    mode: "key",
    background: { kind: "black" },
    keyOf: "out1fill00",
    alwaysOnTop: true,
  },
];

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, media`, () => {
    let assets: AssetInfo[] = [];
    test.beforeEach(async ({ page, context, server }) => {
      assets = await libraryAssets(server);
      await mirrorScreens(page, context, server);
    });

    test("media library: issues with Convert…, list view, import in progress", async ({ page }) => {
      await open(page, "/sources?tab=media", scenario(theme, assets));
      const lib = page.getByRole("listbox", { name: "Media" });
      await expect(lib.getByRole("option")).toHaveCount(5);
      await lib.getByRole("option", { name: /interview-hevc/ }).click();
      await page.getByText("Convert…").first().click();
      await shot(page, `${theme}-110-media-library`);

      await page.getByRole("button", { name: "List" }).click();
      await lib.getByRole("option", { name: /clip-vp9-alpha/ }).click();
      await shot(page, `${theme}-111-media-library-list`);

      await open(
        page,
        "/sources?tab=media",
        scenario(theme, assets, {
          dialogPick: ["D:\\Media\\keynote-recording.mp4", "D:\\Media\\sponsor-loop.webm"],
          importEvents: [
            { afterMs: 50, progress: { index: 0, name: "keynote-recording.mp4", copied: 1_288_490_188, total: 3_435_973_836, done: false, error: null } },
            { afterMs: 60, progress: { index: 1, name: "sponsor-loop.webm", copied: 18_874_368, total: 18_874_368, done: true, error: null } },
          ],
        }),
      );
      await page.getByRole("button", { name: "Import…" }).first().click();
      await expect(page.getByRole("progressbar", { name: "Importing keynote-recording.mp4" })).toBeVisible();
      await shot(page, `${theme}-112-media-import`);
    });

    test("layout editor: media Inspector and Add layer › Media…", async ({ page }) => {
      await open(page, `/screens/${MEDIA_SCREEN}`, scenario(theme, assets));
      await page.getByRole("button", { name: /^Opener/ }).first().click();
      await expect(page.getByRole("heading", { name: "Play range" })).toBeVisible();
      await shot(page, `${theme}-113-media-inspector`);
      await page.getByRole("button", { name: "Add layer" }).click();
      await expect(page.getByRole("menuitem", { name: "Media…" })).toBeVisible();
      await shot(page, `${theme}-114-media-add-layer`);
    });

    test("control: media rows with transport, times and readiness", async ({ page }) => {
      await open(page, "/control", scenario(theme, assets, { selectedScreenId: MEDIA_SCREEN, control: { view: "single", selectedScreenId: MEDIA_SCREEN } }));
      await expect(page.getByRole("button", { name: "Pause Opener" })).toBeVisible();
      await expect(page.getByText("Loading 2/3")).toBeVisible();
      await shot(page, `${theme}-115-control-media`);
      await page.getByRole("button", { name: "Take Sting" }).click();
      await expect(page.getByText("Not loaded on 1 output. Take anyway?")).toBeVisible();
      await shot(page, `${theme}-116-control-take-not-ready`);
    });

    test("preferences: outputs with audio devices", async ({ page }) => {
      await page.setViewportSize({ width: 1100, height: 900 });
      await open(page, "/preferences?section=outputs", {
        ...scenario(theme, assets),
        outputs: AUDIO_OUTPUTS,
        audioDevices: ["Speakers (Realtek(R) Audio)", "Speakers (Focusrite USB Audio)", "DELL U2720Q (Intel(R) Display Audio)"],
      });
      await expect(page.getByRole("combobox", { name: "Audio device for Program fill" })).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-117-outputs-audio`);
    });
  });
}
