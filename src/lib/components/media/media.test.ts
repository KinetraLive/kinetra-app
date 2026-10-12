import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../templates/list/matchers";
import type { AssetImportProgress, AssetInfo, LiveLayerView, MediaInfo, MediaIssue, ServerStatus } from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Set<(p: unknown) => void>>() }));
const api = vi.hoisted(() => ({
  listAssets: vi.fn(),
  deleteAsset: vi.fn(),
  renameAsset: vi.fn(),
  serverStatus: vi.fn(),
  on: vi.fn(),
  mediaControl: vi.fn(),
  importMedia: vi.fn(),
  cancelImport: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e ? e : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/assets", () => api);
vi.mock("$lib/api/media", () => api);
vi.mock("$lib/api/live", () => api);
const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import {
  clampRange,
  codecText,
  elapsedRemaining,
  formatTime,
  kindOfName,
  mediaRect,
  newMediaLayer,
  notReadyQuestion,
  parseTime,
  positionAt,
  readinessText,
  sortIssues,
  worstIssue,
} from "./media";
import { baseName, importer, jobBytes } from "./importer.svelte";
import { confirmTake } from "./take";
import { pickTitle } from "./pick";
import { audioOf, deviceOptions, outputLabels } from "$lib/components/outputs/audio.svelte";
import MediaLibrary from "./MediaLibrary.svelte";
import MediaTransport from "./MediaTransport.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const running: ServerStatus = { state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null };

const info = (over: Partial<MediaInfo> = {}): MediaInfo => ({
  kind: "video",
  container: "mp4",
  durationMs: 65_500,
  width: 1920,
  height: 1080,
  fps: 29.97,
  videoCodec: "h264 High 4.1",
  audioCodec: "aac",
  alpha: false,
  fastStart: true,
  issues: [],
  ...over,
});

const hevc: MediaIssue = {
  code: "unsupported",
  severity: "error",
  platforms: ["obs"],
  message: "Won't play in OBS: HEVC.",
  convert: { label: "Convert to H.264", command: 'ffmpeg -i "clip.mp4" -c:v libx264 -crf 18 "clip-h264.mp4"' },
};
const slow: MediaIssue = {
  code: "slowStart",
  severity: "warning",
  platforms: ["windows", "obs", "vmix"],
  message: "Starts slowly over the network.",
  convert: null,
};

const asset = (name: string, media?: MediaInfo, bytes = 4096): AssetInfo => ({
  name,
  url: `/assets/${encodeURIComponent(name)}`,
  bytes,
  modifiedAt: "2026-10-10T10:00:00Z",
  usedBy: [],
  ...(media ? { media } : {}),
});

let files: AssetInfo[];

function emit(event: string, payload: unknown) {
  for (const h of events.handlers.get(event) ?? []) h(payload);
}

async function settle() {
  for (let i = 0; i < 6; i++) {
    await tick();
    await Promise.resolve();
  }
}

beforeEach(() => {
  vi.clearAllMocks();
  events.handlers.clear();
  toasts.clear();
  server.status = null;
  files = [
    asset("clip.mp4", info({ issues: [slow, hevc] })),
    asset("intro.webm", info({ container: "webm", videoCodec: "vp9", audioCodec: null, alpha: true, durationMs: 4000 })),
    asset("bed.mp3", info({ kind: "audio", container: "mp3", width: null, height: null, fps: null, videoCodec: null, audioCodec: "mp3", durationMs: 180_000 })),
    asset("logo.png", info({ kind: "image", container: "png", durationMs: null, fps: null, videoCodec: null, audioCodec: null, width: 400, height: 200 })),
  ];
  api.on.mockImplementation((event: string, h: Handler) => {
    let set = events.handlers.get(event);
    if (!set) events.handlers.set(event, (set = new Set()));
    set.add(h);
    return () => set.delete(h);
  });
  api.serverStatus.mockResolvedValue(running);
  api.listAssets.mockImplementation(async () => [...files]);
  api.mediaControl.mockResolvedValue(undefined);
  api.cancelImport.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

// ---- Pure helpers -------------------------------------------------------------------------------

describe("media helpers", () => {
  it("kinds by extension", () => {
    expect(kindOfName("a.MP4")).toBe("video");
    expect(kindOfName("/assets/b.webm")).toBe("video");
    expect(kindOfName("c.m4a")).toBe("audio");
    expect(kindOfName("d.jpeg")).toBe("image");
    expect(kindOfName("e.txt")).toBeNull();
  });

  it("times: m:ss, h:mm:ss, hundredths; typed times parse", () => {
    expect(formatTime(5_000)).toBe("0:05");
    expect(formatTime(754_000)).toBe("12:34");
    expect(formatTime(3_723_000)).toBe("1:02:03");
    expect(formatTime(5_204, true)).toBe("0:05.20");
    expect(formatTime(-40)).toBe("0:00");
    expect(parseTime("1:23.5")).toBe(83_500);
    expect(parseTime("83,5")).toBe(83_500);
    expect(parseTime("1:02:03")).toBe(3_723_000);
    expect(parseTime("1:75")).toBeNull();
    expect(parseTime("abc")).toBeNull();
  });

  it("the position follows the renderers' rule: playing from `at`, clamped or wrapped at the out point", () => {
    const m = { inMs: 1000, outMs: 5000, endAction: "hold" as const };
    const play = { state: "playing" as const, at: 10_000, posMs: 1000 };
    // Scheduled ahead: still at the in point before `at`.
    expect(positionAt(play, 9_900, m, 10_000)).toEqual({ pos: 1000, ended: false });
    expect(positionAt(play, 12_000, m, 10_000)).toEqual({ pos: 3000, ended: false });
    expect(positionAt(play, 20_000, m, 10_000)).toEqual({ pos: 5000, ended: true });
    expect(positionAt(play, 15_500, { ...m, endAction: "loop" }, 10_000)).toEqual({ pos: 2500, ended: false });
    expect(positionAt({ state: "paused", at: 0, posMs: 2200 }, 99_999, m, 10_000)).toEqual({ pos: 2200, ended: false });
    // No out point: the file's end.
    expect(positionAt(play, 30_000, { ...m, outMs: null }, 10_000).pos).toBe(10_000);
    expect(elapsedRemaining(play, 12_000, m, 10_000)).toEqual({ elapsed: 2000, remaining: 2000 });
    expect(elapsedRemaining(play, 12_000, { ...m, outMs: null }, null)).toEqual({ elapsed: 2000, remaining: null });
  });

  it("readiness chip and the Take question", () => {
    expect(readinessText(undefined)).toBeNull();
    expect(readinessText({ ready: 3, total: 3, error: null })).toBeNull();
    expect(readinessText({ ready: 0, total: 0, error: null })).toBeNull();
    expect(readinessText({ ready: 2, total: 3, error: null })).toEqual({ text: "Loading 2/3", error: false });
    expect(readinessText({ ready: 1, total: 2, error: "Can't play this codec" })).toEqual({ text: "Can't load on 1 of 2", error: true });
    expect(notReadyQuestion({ ready: 2, total: 3, error: null })).toBe("Not loaded on 1 output. Take anyway?");
    expect(notReadyQuestion({ ready: 0, total: 2, error: null })).toBe("Not loaded on 2 outputs. Take anyway?");
    expect(notReadyQuestion({ ready: 2, total: 2, error: null })).toBeNull();
  });

  it("issues: worst severity, errors first; codec text", () => {
    expect(worstIssue([])).toBeNull();
    expect(worstIssue([slow])).toBe("warning");
    expect(worstIssue([slow, hevc])).toBe("error");
    expect(sortIssues([slow, hevc]).map((i) => i.code)).toEqual(["unsupported", "slowStart"]);
    expect(codecText(info())).toBe("h264 High 4.1 · aac");
  });

  it("new media layers: their own size centered, else the screen; the range clamps to the file", () => {
    const screen = { width: 1920, height: 1080 };
    expect(mediaRect(info({ width: 640, height: 360 }), screen)).toEqual({ x: 640, y: 360, w: 640, h: 360 });
    expect(mediaRect(info(), screen)).toEqual({ x: 0, y: 0, w: 1920, h: 1080 });
    expect(mediaRect(info({ kind: "audio", width: null, height: null }), screen)).toEqual({ x: 0, y: 0, w: 1920, h: 1080 });
    const l = newMediaLayer(asset("Big clip.mp4", info({ width: 640, height: 360 })), screen, "abc");
    expect(l).toMatchObject({ id: "abc", name: "Big clip", kind: "media", media: { asset: "/assets/Big%20clip.mp4", fit: "fit", inMs: 0, outMs: null, volume: 100, muted: false, endAction: "hold", autoplay: true } });
    expect(clampRange(-5, 3000, 10_000)).toEqual({ inMs: 0, outMs: 3000 });
    expect(clampRange(4000, 3000, 10_000)).toEqual({ inMs: 4000, outMs: 4001 });
    expect(clampRange(0, 12_000, 10_000)).toEqual({ inMs: 0, outMs: null });
    expect(clampRange(20_000, null, 10_000)).toEqual({ inMs: 9999, outMs: null });
  });

  it("picker titles; import progress sums", () => {
    expect(pickTitle(["video"], "Clip")).toBe("Choose video for Clip");
    expect(pickTitle(["video", "audio", "image"])).toBe("Choose media");
    expect(baseName("C:\\Media\\a b.mp4")).toBe("a b.mp4");
    expect(baseName("/home/x/c.webm")).toBe("c.webm");
    const p = (name: string, copied: number, total: number): AssetImportProgress => ({ job: "j", index: 0, name, copied, total, done: false, error: null });
    expect(jobBytes({ id: "j", names: ["a", "b"], files: { 0: p("a", 10, 100), 1: p("b", 5, 50) }, cancelling: false })).toEqual({ copied: 15, total: 150 });
  });

  it("audio devices by label: aliases dropped, System default first, a missing saved one kept", () => {
    const d = (deviceId: string, label: string, kind: MediaDeviceKind = "audiooutput") => ({ deviceId, label, kind });
    expect(
      outputLabels([d("default", "Default - Speakers"), d("communications", "Communications - Headset"), d("x1", "Speakers"), d("x2", "Headset"), d("m", "Mic", "audioinput"), d("x3", "")]),
    ).toEqual(["Headset", "Speakers"]);
    expect(deviceOptions(["Speakers"], null)).toEqual([
      { value: "", label: "System default" },
      { value: "Speakers", label: "Speakers" },
    ]);
    expect(deviceOptions([], "USB DAC").at(-1)).toEqual({ value: "USB DAC", label: "USB DAC (not connected)" });
    expect(audioOf({ mode: "fill" })).toEqual({ enabled: true, device: null });
    expect(audioOf({ mode: "key", audio: { enabled: true, device: "X" } })).toEqual({ enabled: false, device: null });
  });
});

// ---- Import -------------------------------------------------------------------------------------

describe("importer", () => {
  it("copies with progress events (early ones too), toasts when done, cancel calls the backend", async () => {
    api.importMedia.mockImplementation(async () => {
      // The first event arrives before the reply.
      emit("asset-import", { job: "j1", index: 0, name: "a.mp4", copied: 0, total: 1000, done: false, error: null });
      return "j1";
    });
    await importer.start(["C:\\in\\a.mp4", "C:\\in\\b.mov"]);
    expect(importer.jobs).toHaveLength(1);
    expect(importer.jobs[0].files[0].total).toBe(1000);
    emit("asset-import", { job: "j1", index: 0, name: "a.mp4", copied: 1000, total: 1000, done: true, error: null });
    expect(importer.busy).toBe(true);
    await importer.cancel("j1");
    expect(api.cancelImport).toHaveBeenCalledWith("j1");
    expect(importer.jobs[0].cancelling).toBe(true);
    emit("asset-import", { job: "j1", index: 1, name: "b.mov", copied: 0, total: 0, done: true, error: "Canceled" });
    expect(importer.busy).toBe(false);
    expect(importer.lastImported).toEqual(["a.mp4"]);
    expect(toasts.list.map((t) => t.message)).toEqual(["Imported “a.mp4”."]);
    // No listener once nothing imports.
    expect(events.handlers.get("asset-import")?.size ?? 0).toBe(0);
  });

  it("a file saved under another name (sanitized, -2) still finishes the job: one row per picked file", async () => {
    api.importMedia.mockImplementation(async () => "jn");
    await importer.start(["C:/in/Video(25).mov"]);
    expect(importer.jobs[0].names).toEqual(["Video(25).mov"]);
    emit("asset-import", { job: "jn", index: 0, name: "Video-25.mov", copied: 0, total: 2600, done: false, error: null });
    expect(importer.jobs[0].names).toEqual(["Video-25.mov"]);
    emit("asset-import", { job: "jn", index: 0, name: "Video-25-2.mov", copied: 2600, total: 2600, done: true, error: null });
    expect(importer.busy).toBe(false);
    expect(importer.lastImported).toEqual(["Video-25-2.mov"]);
  });

  it("a failed file says why", async () => {
    api.importMedia.mockResolvedValue("j2");
    await importer.start(["big.mov"]);
    emit("asset-import", { job: "j2", index: 0, name: "big.mov", copied: 0, total: 0, done: true, error: "Over 8 GB." });
    expect(toasts.list.at(-1)?.message).toBe("Couldn't import “big.mov”: Over 8 GB.");
  });
});

// ---- Library ------------------------------------------------------------------------------------

describe("media library", () => {
  it("lists files with mono duration, flags issues and shows the Convert… command with Copy", async () => {
    server.status = running;
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    render(MediaLibrary, { props: { initial: "/assets/clip.mp4" } });
    await vi.waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(4));
    const opts = screen.getAllByRole("option");
    expect(opts.map((o) => o.dataset.asset)).toEqual(["bed.mp3", "clip.mp4", "intro.webm", "logo.png"]);
    const clip = opts[1];
    expect(clip).toHaveAttribute("aria-selected", "true");
    expect(clip.querySelector(".meta.mono")?.textContent).toBe("1:05");
    expect(within(clip).getByLabelText("Won't play everywhere")).toBeTruthy();
    // Details: facts in mono, issues errors first.
    const details = screen.getByRole("complementary", { name: "Details" });
    expect(within(details).getByText("1920×1080").classList.contains("mono")).toBe(true);
    const issues = within(details).getByRole("list", { name: "Playback issues" });
    expect(within(issues).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      expect.stringContaining("Won't play in OBS: HEVC."),
      expect.stringContaining("Starts slowly over the network."),
    ]);
    await fireEvent.click(within(issues).getByText("Convert…"));
    await fireEvent.click(within(issues).getByRole("button", { name: "Copy command" }));
    expect(writeText).toHaveBeenCalledWith(hevc.convert!.command);
    expect(document.querySelectorAll("[placeholder]")).toHaveLength(0);
  });

  it("filters by kind and selects with the keyboard", async () => {
    render(MediaLibrary);
    await vi.waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(4));
    await fireEvent.click(screen.getByRole("radio", { name: "Audio" }));
    expect(screen.getAllByRole("option").map((o) => o.dataset.asset)).toEqual(["bed.mp3"]);
    await fireEvent.click(screen.getByRole("radio", { name: "All" }));
    const list = screen.getByRole("listbox", { name: "Media" });
    await fireEvent.keyDown(list, { key: "End" });
    await settle();
    expect(screen.getAllByRole("option").at(-1)).toHaveAttribute("aria-selected", "true");
  });

  it("pick mode accepts only the given kinds; Enter chooses", async () => {
    const onchoose = vi.fn();
    render(MediaLibrary, { props: { mode: "pick", accept: ["video"], onchoose } });
    await vi.waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(2));
    const list = screen.getByRole("listbox", { name: "Media" });
    await fireEvent.keyDown(list, { key: "Home" });
    await fireEvent.keyDown(list, { key: "Enter" });
    expect(onchoose).toHaveBeenCalledWith(expect.objectContaining({ name: "clip.mp4" }));
  });

  it("shows imports with progress and Cancel", async () => {
    api.importMedia.mockResolvedValue("j3");
    dialog.open.mockResolvedValue(["C:\\in\\new.mp4"]);
    render(MediaLibrary);
    await vi.waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(4));
    await fireEvent.click(screen.getByRole("button", { name: "Import…" }));
    await vi.waitFor(() => expect(api.importMedia).toHaveBeenCalledWith(["C:\\in\\new.mp4"]));
    emit("asset-import", { job: "j3", index: 0, name: "new.mp4", copied: 512 * 1024, total: 2 * 1024 * 1024, done: false, error: null });
    await settle();
    const bar = screen.getByRole("progressbar", { name: "Importing new.mp4" });
    expect(bar.getAttribute("aria-valuetext")).toBe("512 KB of 2.0 MB");
    await fireEvent.click(screen.getByRole("button", { name: "Cancel import" }));
    expect(api.cancelImport).toHaveBeenCalledWith("j3");
    files.push(asset("new.mp4", info()));
    emit("asset-import", { job: "j3", index: 0, name: "new.mp4", copied: 2 * 1024 * 1024, total: 2 * 1024 * 1024, done: true, error: null });
    await vi.waitFor(() => expect(screen.getAllByRole("option").find((o) => o.dataset.asset === "new.mp4")).toHaveAttribute("aria-selected", "true"));
    expect(screen.queryByRole("progressbar")).toBeNull();
  });
});

// ---- Control transport ---------------------------------------------------------------------------

describe("media transport", () => {
  const view = (state: "playing" | "paused" | "stopped", readiness = { ready: 2, total: 2, error: null as string | null }): LiveLayerView => ({
    id: "m1",
    name: "Opener",
    onAir: true,
    fields: {},
    fieldDefs: [],
    pending: false,
    media: { playback: { state, at: 1_000_000, posMs: 0 }, readiness, durationMs: 10_000 },
  });
  const ctl = (confirm: unknown = null) =>
    ({
      doc: {
        id: "s1",
        layers: [{ id: "m1", name: "Opener", kind: "media", rect: { x: 0, y: 0, w: 10, h: 10 }, media: { asset: "/assets/clip.mp4", fit: "fit", inMs: 0, outMs: null, volume: 100, muted: false, endAction: "hold", autoplay: true } }],
      },
      confirmMedia: confirm,
    }) as never;

  it("Play / Stop / Restart call media_control; the file shows in mono", async () => {
    render(MediaTransport, { props: { ctl: ctl(), screenId: "s1", layer: view("paused"), part: "transport" } });
    await fireEvent.click(screen.getByRole("button", { name: "Play Opener" }));
    await fireEvent.click(screen.getByRole("button", { name: "Restart Opener" }));
    await fireEvent.click(screen.getByRole("button", { name: "Stop Opener" }));
    expect(api.mediaControl.mock.calls).toEqual([
      ["s1", "m1", "play"],
      ["s1", "m1", "restart"],
      ["s1", "m1", "stop"],
    ]);
    cleanup();
    render(MediaTransport, { props: { ctl: ctl(), screenId: "s1", layer: view("paused"), part: "file" } });
    expect(screen.getByText("clip.mp4").classList.contains("mono")).toBe(true);
  });

  it("schedules nothing while paused; ticks once a second while playing", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame", "Date"] });
    vi.setSystemTime(1_000_000);
    const { rerender } = render(MediaTransport, { props: { ctl: ctl(), screenId: "s1", layer: view("paused"), part: "transport" } });
    await tick();
    // Whatever the test environment has pending (none of it ours).
    const base = vi.getTimerCount();
    await rerender({ ctl: ctl(), screenId: "s1", layer: view("playing"), part: "transport" });
    await tick();
    expect(vi.getTimerCount()).toBeGreaterThan(base);
    vi.advanceTimersByTime(3_100);
    await tick();
    expect(document.querySelector(".time")?.textContent?.replace(/s+/g, " ")).toBe("0:03 / −0:07");
    expect(screen.getByRole("button", { name: "Pause Opener" })).toBeTruthy();
    await rerender({ ctl: ctl(), screenId: "s1", layer: view("paused"), part: "transport" });
    await tick();
    // Paused: nothing of ours stays scheduled (a few ms for the UI library to settle).
    vi.advanceTimersByTime(500);
    expect(vi.getTimerCount()).toBe(base);
  });

  it("readiness: Loading n/m, an error in the warning style; Control's take asks once", async () => {
    const c = ctl();
    render(MediaTransport, { props: { ctl: c, screenId: "s1", layer: view("paused", { ready: 2, total: 3, error: null }), part: "transport" } });
    expect(screen.getByRole("status").textContent).toBe("Loading 2/3");
    expect((c as { confirmMedia: unknown }).confirmMedia).toBe(confirmTake);
    cleanup();
    render(MediaTransport, { props: { ctl: c, screenId: "s1", layer: view("paused", { ready: 1, total: 2, error: "Can't play" }), part: "transport" } });
    expect(screen.getByRole("status").classList.contains("error")).toBe(true);
  });

  it("confirmTake: at once when loaded everywhere, else Take anyway / Cancel", async () => {
    expect(await confirmTake(view("paused"))).toBe(true);
    let p = confirmTake(view("paused", { ready: 2, total: 3, error: null }));
    await settle();
    expect(screen.getByText("Not loaded on 1 output. Take anyway?")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Take anyway" }));
    expect(await p).toBe(true);
    await settle();
    p = confirmTake(view("paused", { ready: 0, total: 2, error: null }));
    await settle();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await p).toBe(false);
  });
});

// ---- MIDI / Flows / palette wording ---------------------------------------------------------------

import { chipKey, command, groupOf, targetOptions, targetText } from "$lib/components/midi/model";
import { stepText, triggerText } from "$lib/components/flows/linear";
import MediaInspector from "./MediaInspector.svelte";

describe("media in MIDI targets and flows", () => {
  const screens = [
    {
      screen: { id: "s1", name: "Main", width: 1920, height: 1080, background: "transparent" },
      layers: [
        { id: "m1", name: "Opener", onAir: false, fields: {}, fieldDefs: [], pending: false, media: { playback: { state: "stopped" as const, at: 0, posMs: 0 }, readiness: { ready: 0, total: 0, error: null }, durationMs: 10_000 } },
        { id: "t1", name: "Lower", onAir: false, fields: {}, fieldDefs: [], pending: false },
      ],
    },
  ];

  it("media layers offer Play / Pause / Stop as layer commands with chips", () => {
    const k = { screens, flows: [], sources: [] } as never;
    const labels = targetOptions(k).map((o) => o.label);
    expect(labels).toContain("Play Opener (Main)");
    expect(labels).toContain("Stop Opener (Main)");
    expect(labels.some((l) => l.startsWith("Play Lower"))).toBe(false);
    const t = command("media.pause:s1/m1");
    expect(targetText(t, k)).toBe("Pause Opener (Main)");
    expect(groupOf(t)).toBe("layers");
    expect(chipKey(t)).toBe("s1/m1");
  });

  it("flows read in plain words", () => {
    const k = { screens: new Map([["s1", screens[0]]]), sources: new Map(), flows: new Map() } as never;
    expect(triggerText({ type: "trigger.mediaEnded", config: { screenId: "s1", layerId: "m1" } }, k)).toBe("Opener reaches its end");
    expect(stepText({ type: "action.media", config: { screenId: "s1", layerId: "m1", action: "restart" } }, k)).toBe("Restart Opener");
    expect(stepText({ type: "action.media", config: { screenId: "s1", layerId: "m1", action: "seek", ms: 65_500 } }, k)).toBe("Go to 1:05.5 in Opener");
  });
});

// ---- Inspector -----------------------------------------------------------------------------------

describe("media inspector", () => {
  const layer = (over: Record<string, unknown> = {}) => ({
    id: "m1",
    name: "Opener",
    kind: "media" as const,
    rect: { x: 0, y: 0, w: 1920, h: 1080 },
    media: { asset: "/assets/clip.mp4", fit: "fit" as const, inMs: 0, outMs: null, volume: 80, muted: false, endAction: "hold" as const, autoplay: true, ...over },
  });

  it("file facts, fit, typed in / out, volume, mute, end action and autoplay patch `media`", async () => {
    const onpatch = vi.fn();
    render(MediaInspector, { props: { layer: layer(), screenId: "s1", live: null, onpatch } });
    await vi.waitFor(() => expect(screen.getByText(/1:05\.50 · 1920×1080/)).toBeTruthy());
    await fireEvent.click(screen.getByRole("radio", { name: "Fill" }));
    expect(onpatch).toHaveBeenLastCalledWith({ media: expect.objectContaining({ fit: "fill" }) });
    const inBox = screen.getByLabelText("In") as HTMLInputElement;
    await fireEvent.input(inBox, { target: { value: "0:02.5" } });
    await fireEvent.keyDown(inBox, { key: "Enter" });
    expect(onpatch).toHaveBeenLastCalledWith({ media: expect.objectContaining({ inMs: 2500, outMs: null }) });
    const outBox = screen.getByLabelText("Out") as HTMLInputElement;
    expect(outBox.value).toBe("1:05.50");
    await fireEvent.input(outBox, { target: { value: "nope" } });
    expect(outBox.getAttribute("aria-invalid")).toBe("true");
    await fireEvent.input(outBox, { target: { value: "30" } });
    await fireEvent.blur(outBox);
    expect(onpatch).toHaveBeenLastCalledWith({ media: expect.objectContaining({ outMs: 30_000 }) });
    await fireEvent.input(screen.getByRole("slider", { name: "Volume" }), { target: { value: "55" } });
    expect(onpatch).toHaveBeenLastCalledWith({ media: expect.objectContaining({ volume: 55 }) });
    await fireEvent.click(screen.getByRole("button", { name: "Mute" }));
    expect(onpatch).toHaveBeenLastCalledWith({ media: expect.objectContaining({ muted: true }) });
    await fireEvent.change(screen.getByLabelText("At the end"), { target: { value: "loop" } });
    expect(onpatch).toHaveBeenLastCalledWith({ media: expect.objectContaining({ endAction: "loop" }) });
    await fireEvent.click(screen.getByRole("switch", { name: "Play on take" }));
    expect(onpatch).toHaveBeenLastCalledWith({ media: expect.objectContaining({ autoplay: false }) });
  });

  it("an image has no play range or sound; a missing file says so", async () => {
    render(MediaInspector, { props: { layer: layer({ asset: "/assets/logo.png" }), screenId: "s1", live: null, missing: true, onpatch: vi.fn() } });
    await settle();
    expect(screen.queryByText("Play range")).toBeNull();
    expect(screen.queryByText("Sound")).toBeNull();
    expect(screen.getByText("Picture")).toBeTruthy();
    expect(screen.getByText("This file is no longer in the project. Choose another file.")).toBeTruthy();
  });
});
