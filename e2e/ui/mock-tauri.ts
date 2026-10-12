// Mock Tauri backend for the UI screenshot harness. `installMockTauri` is serialized into the page
// with `page.addInitScript(installMockTauri, scenario)`, so it must be self-contained (no imports,
// no closures over module scope). It defines `window.__TAURI_INTERNALS__` (invoke,
// transformCallback, unregisterCallback, metadata) and the event plugin internals, and answers the
// commands the app calls with the scenario's data. Lives only in the harness, never in the app.
//
// Templates live in memory (`templates`, built by the spec from the real preset files). The
// preview-screen commands also drive the fixture output server through `window.__kinetraFixture`
// (a Playwright binding the spec exposes): `preview_template` → `template <presetId>`,
// `preview_show`/`preview_hide` → `show`/`hide`, `preview_draft` values → `fields`/`vars`. The
// spec bridges the `__preview` screen's WebSocket to the fixture's `test` screen, so the preview
// iframe shows the real renderer drawing the real preset.
//
// Screens live in memory too (`screenDocs`); every change is mirrored to the fixture's real
// project (`savescreen`, `layershow`/`layerhide`, `rect`), and the spec routes those screens'
// WebSockets to the fixture's screen of the same id, so the canvas is the real output.

import type {
  ApiStatus,
  ApplyStyleResult,
  AssetImportProgress,
  AssetInfo,
  LiveLayerView,
  BrandKit,
  ControlsDoc,
  ControlViewState,
  DisplayInfo,
  FlowInfo,
  FlowTemplateInfo,
  MidiDevice,
  MidiLearned,
  MidiPrefs,
  OscSettings,
  OscStatus,
  OutputConfig,
  OutputStatus,
  PresetInfo,
  RecipeInstance,
  ScheduleDoc,
  ScheduleView,
  ScreenDoc,
  ScreenEditorView,
  SourceDoc,
  StyleInfo,
  TemplateDoc,
  UpdateCheck,
  LicenseView,
  PollData,
  PollQuota,
  PollSourceDoc,
} from "../../src/lib/types";

export interface MockScenario {
  theme: "light" | "dark";
  /** Screens returned by `list_screens` (the test screen is added automatically). */
  screens: { id: string; name: string; width: number; height: number; onAir: boolean }[];
  lanAccess: boolean;
  selectedScreenId: string | null;
  /** The test screen's lower third is on air (matches the fixture server's state). */
  testOnAir: boolean;
  /** No project open: the app shows the project picker. */
  noProject?: boolean;
  /** Recent projects for the picker. */
  recents?: { path: string; name: string; lastOpenedAt: string; missing: boolean }[];
  /** The project's templates (full documents). */
  templates?: TemplateDoc[];
  /** Built-in presets for `list_presets` and `create_template`. */
  presets?: PresetInfo[];
  /**
   * Project screens. When set, `list_screens` and the screen commands use these instead of
   * `screens`, and every change is mirrored to the fixture server (`savescreen`, `layershow`, ...)
   * with template ids mapped through `fixtureTemplateIds`, so the canvas shows the real renderer.
   */
  screenDocs?: ScreenDoc[];
  fixtureTemplateIds?: Record<string, string>;
  /** Layers shown on air at start (`<screenId>/<layerId>`). */
  onAirLayers?: string[];
  /** Layers marked "Changes on next take" (`<screenId>/<layerId>`). */
  pendingLayers?: string[];
  /** Control layout overrides. */
  control?: Partial<ControlViewState>;
  /** Text sources, mirrored into the fixture with ids mapped through `fixtureSourceIds`. */
  sourceDocs?: SourceDoc[];
  fixtureSourceIds?: Record<string, string>;
  /** Flows: cues, rules, recipe flows. Run emits `flow-state` (running, then finished). */
  flows?: FlowInfo[];
  /** Built-in flow templates for `list_flow_templates`; `add_flow_template` adds their
   *  flows in memory (a Cue or automatic trigger and a Take on the first layer slot). */
  flowTemplates?: FlowTemplateInfo[];
  /** Schedule sources, mirrored into the fixture (ids through `fixtureSourceIds`): the view
   *  (statuses, NOW, UP NEXT) and operator actions come from the fixture's real schedule engine. */
  scheduleDocs?: ScheduleDoc[];
  /** Images in assets/, imported into the fixture so thumbnails load from it. */
  assets?: AssetInfo[];
  /** Installed recipes. */
  recipeInstances?: RecipeInstance[];
  /** Style library: `list_styles` (default: none), the project's style and brand kit. */
  styles?: StyleInfo[];
  projectStyle?: string | null;
  brand?: BrandKit;
  /** Layout editor view options (ui-state `screenEditor`); defaults otherwise. */
  screenEditor?: Partial<ScreenEditorView>;
  /**
   * Studio mode: each screen's Preview diff at start (screen id → cues; `fields` staged values,
   * `programChanged` the marker) and a T-bar position. With project screens the Preview is mirrored
   * into the fixture as screen "pv" + id.slice(2), which /screen/<id>/preview shows (harness.ts).
   */
  studioCues?: Record<string, { layerId: string; visible: boolean | null; fields?: Record<string, string>; programChanged?: boolean }[]>;
  studioTbar?: Record<string, number>;
  /**
   * API sources (`sourceDocs` with kind "api"): the fetch status per source id (default: Live,
   * fetched 4 s ago) and screens that use them besides bindings (Control's sources strip).
   * `test_api_source` answers a JSON league table or, for CSV, a weather table.
   */
  apiStatus?: Record<string, ApiStatus>;
  apiUsedBy?: Record<string, string[]>;
  /** The machine's key bindings (prefs.json `keybindings`: command id → keys). */
  keybindings?: Record<string, string[]>;
  /** MIDI / OSC control: the machine's devices and their prefs, the project's mappings
   * (controls.json) and OSC settings. While a learn session is open, `midiLearnNext` is sent as
   * `midi-learn` after `midiLearnDelayMs` (default 600; null = never: the session waits). */
  midiDevices?: MidiDevice[];
  midiPrefs?: MidiPrefs;
  controls?: ControlsDoc;
  osc?: OscSettings;
  /** The OSC listener (`osc_status`); default listening on 9000, local only. */
  oscStatus?: OscStatus;
  midiLearnNext?: MidiLearned;
  midiLearnDelayMs?: number | null;
  /** Preferences > About: what `check_for_update` answers (after ~400 ms); "fail" rejects with
   * code "updateCheck". Default: up to date. */
  update?: UpdateCheck | "fail";
  /** prefs.json `uiMode` (absent: the app treats it as Advanced). */
  prefsUiMode?: "simple" | "advanced";
  /** The core's license view (`license_view`), merged over a signed-in Pro default. */
  license?: Partial<LicenseView>;
  /** Live polls: the account's polls (`list_polls`), the quota (null: no access) and this
   * project's poll sources (test or linked). */
  polls?: PollData[];
  pollQuota?: PollQuota | null;
  pollSources?: PollSourceDoc[];
  /**
   * Display outputs: the connected displays (`list_displays`), the machine's outputs (prefs.json
   * `outputs`) and per-output status overrides. Without an override an output's status is
   * computed like the core plans it: off, displayMissing (its display isn't in `displays`),
   * noProject, screenMissing, else showing; a key pair is frame-locked when the two displays are
   * adjacent with the same size and refresh rate. Prefs saves and test patterns emit
   * `outputs-changed`.
   */
  displays?: DisplayInfo[];
  outputs?: OutputConfig[];
  outputStatus?: Record<string, Partial<OutputStatus>>;
  /** Media layers' live view (`<screenId>/<layerId>` → playback, readiness, duration); default
   * stopped, nothing connected. `media_control` updates it and emits `live-changed`. */
  mediaLive?: Record<string, NonNullable<LiveLayerView["media"]>>;
  /** `import_media`: the progress events sent for the job (ms after the call, then the event);
   * a file whose last event is `done` without an error is added to the assets. */
  importEvents?: { afterMs: number; progress: Omit<AssetImportProgress, "job">; asset?: AssetInfo }[];
  /** Audio output labels `navigator.mediaDevices.enumerateDevices()` lists (Preferences › Outputs). */
  audioDevices?: string[];
  /** What the system file dialog returns (`plugin:dialog|open`); default null (cancelled). */
  dialogPick?: string[];
}

export function installMockTauri(s: MockScenario): void {
  const w = window as unknown as Record<string, unknown>;

  try {
    localStorage.setItem("kinetra.theme", s.theme);
  } catch {
    // ignore
  }
  if (s.audioDevices && navigator.mediaDevices) {
    const list = ["default", ...s.audioDevices].map((label, i) => ({
      deviceId: i === 0 ? "default" : `dev${i}`,
      groupId: "g",
      kind: "audiooutput",
      label: i === 0 ? `Default - ${s.audioDevices![0] ?? ""}` : label,
      toJSON: () => ({}),
    }));
    navigator.mediaDevices.enumerateDevices = async () => list as unknown as MediaDeviceInfo[];
  }

  const project = {
    path: "D:\\Shows\\Evening News",
    name: "Evening News",
    createdAt: "2026-09-14T17:30:00Z",
    settings: {
      port: 7770,
      lanAccess: s.lanAccess,
      defaultResolution: { width: 1920, height: 1080 },
      style: s.projectStyle ?? null,
      brand: s.brand ?? {},
      ...(s.osc ? { osc: s.osc } : {}),
    } as Record<string, unknown>,
  };
  let prefs = {
    theme: s.theme,
    backup: { enabled: true, intervalMinutes: 15, retention: 10 },
    keybindings: s.keybindings ?? {},
    ...(s.midiPrefs ? { midi: s.midiPrefs } : {}),
    ...(s.prefsUiMode ? { uiMode: s.prefsUiMode } : {}),
    ...(s.outputs ? { outputs: s.outputs } : {}),
  } as Record<string, unknown>;
  let ui = {
    sidebarPinned: false,
    control: {
      view: "grid",
      gridColumns: 2,
      selectedScreenId: s.selectedScreenId,
      hiddenScreenIds: [],
      liveEdit: "immediate",
      hotkeys: {},
      automationOff: false,
      ...s.control,
    },
    screenEditor: {
      snap: true,
      guides: true,
      grid: false,
      gridSize: 16,
      actionSafe: false,
      titleSafe: false,
      centerCross: false,
      customSafe: true,
      crop: "none",
      ...s.screenEditor,
    },
  };
  const status = {
    state: "running",
    port: 7770,
    url: "http://127.0.0.1:7770",
    lanUrl: s.lanAccess ? "http://192.168.1.42:7770" : null,
  };
  const testLayer = {
    id: "lower-third",
    name: "Lower third",
    onAir: false,
    fields: { name: "Ada Lovelace", title: "Analyst & Mathematician" },
    fieldDefs: [
      { key: "name", label: "Name", type: "text", default: "" },
      { key: "title", label: "Title", type: "text", default: "" },
    ],
  };
  const testScreen = {
    screen: { id: "test", name: "Output test", width: 1920, height: 1080, background: "transparent" },
    layers: [{ ...testLayer, onAir: s.testOnAir }],
  };
  const screens = [
    testScreen,
    ...s.screens.map((x) => ({
      screen: { id: x.id, name: x.name, width: x.width, height: x.height, background: "transparent" },
      layers: [{ ...testLayer, id: `${x.id}-l1`, onAir: x.onAir }],
    })),
  ];
  const now = Date.now();
  const iso = (msAgo: number) => new Date(now - msAgo).toISOString();
  const backups = [
    { id: "b1", createdAt: iso(12 * 60_000), reason: "timer", files: 9, bytes: 18_432 },
    { id: "b2", createdAt: iso(3 * 3_600_000), reason: "pre-import", files: 9, bytes: 18_120 },
    { id: "b3", createdAt: iso(26 * 3_600_000), reason: "open", files: 7, bytes: 12_800 },
  ];

  // ---- Templates ------------------------------------------------------------------------------
  type Doc = TemplateDoc;
  const docs = new Map<string, Doc>((s.templates ?? []).map((d) => [d.id, d]));
  const presets = s.presets ?? [];
  /**
   * Sends a command to the fixture output server; resolves with its reply (the text after "OK "),
   * or null without the binding (e.g. in iframes) or on an error.
   */
  const fixture = async (line: string): Promise<string | null> => {
    const f = w.__kinetraFixture;
    if (typeof f !== "function") return null;
    try {
      return await (f as (l: string) => Promise<string>)(line);
    } catch {
      // The preview is decoration in the harness; the UI must not fail on it.
      return null;
    }
  };
  const summary = (d: Doc) => ({
    id: d.id,
    name: d.name,
    description: d.description,
    preset: d.preset,
    fieldCount: d.fields.length,
    variableCount: d.variables.length,
    variantCount: d.variants?.length ?? 1,
    updatedAt: d.updatedAt,
  });
  const newId = () => Math.random().toString(36).slice(2, 12).padEnd(10, "x");
  const find = (id: unknown): Doc => {
    const d = docs.get(String(id));
    if (!d) throw { code: "notFound", message: "Template not found" };
    return d;
  };
  const findVariant = (d: Doc, id: unknown) => {
    const v = d.variants.find((x) => x.id === id);
    if (!v) throw { code: "notFound", message: "Variant not found" };
    return v;
  };
  /** Applies a change to a template's variants; returns the whole updated document. */
  const updateVariants = (id: unknown, change: (d: Doc) => Partial<Doc>): Doc => {
    const d = find(id);
    const next = { ...d, ...change(d), updatedAt: new Date().toISOString() };
    docs.set(d.id, next);
    return next;
  };
  const preview = { doc: null as Doc | null, onAir: false, values: {} as Record<string, string> };
  const previewLayer = () => ({
    id: "template",
    name: preview.doc?.name ?? "Template",
    onAir: preview.onAir,
    pending: false,
    fields: preview.values,
    fieldDefs: preview.doc?.fields ?? [],
  });

  // ---- Screens ----------------------------------------------------------------------------
  type SDoc = ScreenDoc;
  const sdocs = new Map<string, SDoc>((s.screenDocs ?? []).map((d) => [d.id, d]));
  const onAir = new Set(s.onAirLayers ?? []);
  const pending = new Set(s.pendingLayers ?? []);
  const tmap = s.fixtureTemplateIds ?? {};
  const smap = s.fixtureSourceIds ?? {};
  const toFixture = (d: SDoc) => ({
    ...d,
    // The fixture's templates are the presets (one variant each): the variant stays in the mock.
    layers: d.layers.map(({ variant: _variant, ...l }) => ({
      ...l,
      templateId: tmap[l.templateId] ?? l.templateId,
      ...(l.bindings
        ? {
            bindings: Object.fromEntries(
              Object.entries(l.bindings).map(([k, b]) => [k, { ...b, sourceId: smap[b.sourceId] ?? b.sourceId }]),
            ),
          }
        : {}),
    })),
  });

  // ---- Sources -----------------------------------------------------------------------------
  const srcs = new Map<string, SourceDoc>((s.sourceDocs ?? []).map((d) => [d.id, d]));
  let flows: FlowInfo[] = s.flows ?? [];
  const findFlow = (id: unknown): FlowInfo => {
    const f = flows.find((x) => x.id === id);
    if (!f) throw { code: "notFound", message: "Flow not found" };
    return f;
  };
  const putFlow = (f: FlowInfo): FlowInfo => {
    flows = flows.some((x) => x.id === f.id) ? flows.map((x) => (x.id === f.id ? f : x)) : [...flows, f];
    return f;
  };
  const findSource = (id: unknown): SourceDoc => {
    const d = srcs.get(String(id));
    if (!d) throw { code: "notFound", message: "Source not found" };
    return d;
  };
  const currentRow = (d: SourceDoc): Record<string, string> => {
    const row = d.rows[d.index] ?? {};
    return Object.fromEntries(d.columns.map((c) => [c.key, d.rows.length ? (row[c.key] ?? "") : ""]));
  };
  // ---- Schedules ---------------------------------------------------------------------------
  // The document lives here; the view (statuses, NOW, UP NEXT) is the fixture's real engine's,
  // read with `scheduleview` (source id mapped back) and cached for the bound-field values.
  const scheds = new Map<string, ScheduleDoc>((s.scheduleDocs ?? []).map((d) => [d.id, d]));
  const pollDocs = new Map<string, PollSourceDoc>((s.pollSources ?? []).map((d) => [d.id, d]));
  const pollSummary = (d: PollSourceDoc) => {
    const data = d.test
      ? { status: d.test.status, total: d.test.votes.reduce((a, b) => a + b, 0) }
      : { status: d.cache?.status ?? "draft", total: d.cache?.totalVotes ?? 0 };
    return {
      id: d.id,
      name: d.name,
      kind: "poll" as const,
      rowCount: d.test ? d.test.options.length : (d.cache?.options.length ?? 0),
      index: 0,
      current: {},
      columns: [],
      usedBy: [],
      updatedAt: d.updatedAt,
      auto: null,
      nextAt: null,
      nowId: null,
      nextId: null,
      poll: {
        test: !!d.test,
        pollId: d.link?.id ?? null,
        code: d.link?.code ?? (d.test ? "TEST00" : null),
        conn: d.test ? ("test" as const) : ("live" as const),
        state: data.status,
        totalVotes: data.total,
        simulating: !!d.test && d.test.status === "open",
      },
    };
  };
  /** A QR-looking image for screenshots (the real one is made by the core). */
  const fakeQr = (text: string) => {
    let h = 2166136261;
    for (const c of text) h = Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0;
    const n = 29;
    const finder = (x: number, y: number) =>
      [[0, 0], [n - 7, 0], [0, n - 7]].some(([fx, fy]) => {
        const dx = x - fx, dy = y - fy;
        if (dx < 0 || dy < 0 || dx > 6 || dy > 6) return false;
        return dx === 0 || dy === 0 || dx === 6 || dy === 6 || (dx >= 2 && dx <= 4 && dy >= 2 && dy <= 4);
      });
    const inFinderArea = (x: number, y: number) => (x < 8 && y < 8) || (x > n - 9 && y < 8) || (x < 8 && y > n - 9);
    let path = "";
    for (let y = 0; y < n; y++) {
      for (let x = 0; x < n; x++) {
        h = Math.imul(h ^ (x * 31 + y), 2654435761) >>> 0;
        const dark = inFinderArea(x, y) ? finder(x, y) : (h >>> 7) % 2 === 0;
        if (dark) path += `M${x + 4},${y + 4}h1v1h-1z`;
      }
    }
    const svg = `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 37 37' shape-rendering='crispEdges'><rect width='37' height='37' fill='#fff'/><path fill='#000' d='${path}'/></svg>`;
    return "data:image/svg+xml," + encodeURIComponent(svg);
  };
  const views = new Map<string, ScheduleView>();
  const fetchView = async (id: string): Promise<ScheduleView> => {
    const reply = await fixture(`scheduleview ${smap[id] ?? id}`);
    if (!reply) throw { code: "notFound", message: "Schedule not found" };
    const v = { ...(JSON.parse(reply) as ScheduleView), sourceId: id };
    views.set(id, v);
    return v;
  };
  const scheduleCell = (d: ScheduleDoc, column: string, row: string | undefined): string => {
    const v = views.get(d.id);
    const id = row === "now" ? v?.nowId : v?.nextId;
    const it = d.items.find((x) => x.id === id);
    const iv = v?.items.find((x) => x.id === id);
    if (!it || !iv) return "";
    const hhmm = (t: string) => t.slice(11, 16);
    const labels: Record<string, string> = d.labels as unknown as Record<string, string>;
    switch (column) {
      case "title":
      case "subtitle":
      case "location":
        return it[column];
      case "start":
        return hhmm(it.start);
      case "end":
        return hhmm(iv.end);
      case "new_start":
        return iv.moved ? hhmm(iv.start) : "";
      case "status":
        return iv.status;
      case "label":
        return labels[iv.status] ?? "";
      default:
        return "";
    }
  };
  const scheduleSummary = async (d: ScheduleDoc) => {
    const v = await fetchView(d.id).catch(() => views.get(d.id));
    return {
      id: d.id,
      name: d.name,
      kind: "schedule",
      rowCount: d.items.length,
      index: 0,
      current: {},
      columns: [],
      usedBy: usedBy(d.id),
      updatedAt: d.updatedAt,
      auto: null,
      nextAt: null,
      nowId: v?.nowId ?? null,
      nextId: v?.nextId ?? null,
    };
  };
  const findSchedule = (id: unknown): ScheduleDoc => {
    const d = scheds.get(String(id));
    if (!d) throw { code: "notFound", message: "Source not found" };
    return d;
  };

  // ---- Recipes and assets ------------------------------------------------------------------
  const RECIPES = [
    {
      id: "event-schedule",
      name: "Event schedule",
      description:
        "A run of show with a board, Now & Next and a countdown.",
      panel: "schedule",
      version: 1,
      templates: ["schedule-board", "now-next", "next-up"],
    },
    {
      id: "sponsor-slideshow",
      name: "Sponsor slideshow",
      description:
        "A corner card cycling through your sponsors.",
      panel: "slideshow",
      version: 1,
      templates: ["sponsor-slideshow"],
    },
    {
      id: "sponsor-wall",
      name: "Sponsor wall",
      description:
        "A full-screen grid of sponsor logos, highlighted in turn.",
      panel: "slideshow",
      version: 1,
      templates: ["sponsor-wall"],
    },
    {
      id: "countdown-to-doors",
      name: "Countdown to doors",
      description:
        "A full-screen countdown to the next schedule item.",
      panel: "countdown",
      version: 1,
      templates: ["countdown"],
    },
  ] as const;
  let instances: RecipeInstance[] = s.recipeInstances ?? [];
  const findInstance = (id: unknown): RecipeInstance => {
    const r = instances.find((x) => x.id === id);
    if (!r) throw { code: "notFound", message: "Recipe not found" };
    return r;
  };
  let assets: AssetInfo[] = s.assets ?? [];

  const usedBy = (id: string) =>
    [...sdocs.values()]
      .filter((sd) => sd.layers.some((l) => Object.values(l.bindings ?? {}).some((b) => b.sourceId === id)))
      .map((sd) => sd.id)
      .sort();
  const apiStatus = new Map<string, ApiStatus>(Object.entries(s.apiStatus ?? {}));
  const apiOf = (d: SourceDoc): ApiStatus | null =>
    d.kind !== "api"
      ? null
      : (apiStatus.get(d.id) ?? {
          state: d.poll.paused ? "paused" : "ok",
          lastOkAt: Date.now() - 4000,
          lastAttemptAt: Date.now() - 4000,
          nextAt: d.poll.paused ? null : Date.now() + d.poll.intervalMs - 4000,
          httpStatus: 200,
          message: null,
          rows: d.rows.length,
        });
  const sourceSummary = (d: SourceDoc) => ({
    id: d.id,
    name: d.name,
    kind: d.kind,
    rowCount: d.rows.length,
    index: d.index,
    current: currentRow(d),
    columns: d.columns,
    usedBy: [...new Set([...usedBy(d.id), ...(s.apiUsedBy?.[d.id] ?? [])])].sort(),
    api: apiOf(d),
    format: d.kind === "api" ? d.format : null,
    nowId: null,
    nextId: null,
    updatedAt: d.updatedAt,
    auto: d.auto ?? null,
    // Armed (a bound layer on air): the next step is due part-way through the interval.
    nextAt:
      d.auto && [...sdocs.values()].some((sd) =>
        sd.layers.some(
          (l) => onAir.has(`${sd.id}/${l.id}`) && Object.values(l.bindings ?? {}).some((b) => b.sourceId === d.id),
        ),
      )
        ? Date.now() + Math.round(d.auto.everyMs * 0.6)
        : null,
  });
  const findScreen = (id: unknown): SDoc => {
    const d = sdocs.get(String(id));
    if (!d) throw { code: "notFound", message: "Screen not found" };
    return d;
  };
  const mediaLive: Record<string, NonNullable<LiveLayerView["media"]>> = { ...s.mediaLive };
  const layerView = (d: SDoc, l: SDoc["layers"][number]) => {
    if (l.kind === "media") {
      const key = `${d.id}/${l.id}`;
      mediaLive[key] ??= { playback: { state: "stopped", at: 0, posMs: 0 }, readiness: { ready: 0, total: 0, error: null }, durationMs: null };
      return { id: l.id, name: l.name, onAir: onAir.has(key), pending: pending.has(key), fields: {}, fieldDefs: [], media: mediaLive[key] };
    }
    const t = docs.get(l.templateId);
    return {
      id: l.id,
      name: l.name,
      onAir: onAir.has(`${d.id}/${l.id}`),
      pending: pending.has(`${d.id}/${l.id}`),
      // Bound fields show the source's current row, as the backend resolves them.
      fields: {
        ...l.fields,
        ...Object.fromEntries(
          Object.entries(l.bindings ?? {}).flatMap(([k, b]) => {
            const src = srcs.get(b.sourceId);
            if (src) return [[k, currentRow(src)[b.column] ?? ""]];
            const sch = scheds.get(b.sourceId);
            return sch ? [[k, scheduleCell(sch, b.column, b.row)]] : [];
          }),
        ),
      },
      fieldDefs: t?.fields ?? [],
      variant: variantShown(l),
    };
  };
  /** The variant a layer shows: its own when the template has it, else the default. */
  const variantShown = (l: SDoc["layers"][number]) => {
    const t = docs.get(l.templateId);
    if (!t) return "";
    return l.variant && t.variants?.some((v) => v.id === l.variant) ? l.variant : (t.defaultVariant ?? "default");
  };
  const screenView = (d: SDoc) => ({
    screen: { id: d.id, name: d.name, width: d.width, height: d.height, background: d.background },
    layers: d.layers.filter((l) => l.kind === "media" || docs.has(l.templateId)).map((l) => layerView(d, l)),
  });
  const screenState = (d: SDoc) => ({
    doc: d,
    live: screenView(d),
    warnings: [
      ...d.layers
        .filter((l) => l.kind !== "media" && !docs.has(l.templateId))
        .map((l) => ({ layerId: l.id, code: "templateMissing", message: "Template not found" })),
      ...d.layers
        .filter((l) => l.kind !== "media" && docs.has(l.templateId) && l.variant && variantShown(l) !== l.variant)
        .map((l) => ({ layerId: l.id, code: "variantMissing", message: `Variant "${l.variant}" not found` })),
    ],
  });
  const screenSummary = (d: SDoc) => ({
    id: d.id,
    name: d.name,
    width: d.width,
    height: d.height,
    background: d.background,
    layerCount: d.layers.length,
    updatedAt: d.updatedAt,
  });
  const putScreen = async (d: SDoc) => {
    sdocs.set(d.id, d);
    await fixture(`savescreen ${JSON.stringify(toFixture(d))}`);
  };
  const setOnAir = async (a: Record<string, unknown>, on: boolean) => {
    const d = findScreen(a.screenId);
    const l = d.layers.find((x) => x.id === a.layerId);
    if (!l) throw { code: "notFound", message: "Layer not found" };
    const key = `${d.id}/${l.id}`;
    if (on) onAir.add(key);
    else onAir.delete(key);
    pending.delete(key);
    await fixture(`${on ? "layershow" : "layerhide"} ${d.id} ${l.id}`);
    return layerView(d, l);
  };
  type Cue = { visible: boolean | null; fields?: Record<string, string>; programChanged: boolean };
  type Diff = { rev: number; cues: Map<string, Cue>; tbar: number | null };
  const diffs = new Map<string, Diff>(
    Object.entries(s.studioCues ?? {}).map(([id, list]) => [
      id,
      {
        rev: 1,
        tbar: s.studioTbar?.[id] ?? null,
        cues: new Map(list.map((c) => [c.layerId, { visible: c.visible, fields: c.fields, programChanged: c.programChanged ?? false }])),
      },
    ]),
  );
  const mirrored = new Set<string>();
  const studioOn = () => (ui.control as { studio?: { enabled: boolean } }).studio?.enabled === true;
  const diffOf = (id: string): Diff => {
    let d = diffs.get(id);
    if (!d) diffs.set(id, (d = { rev: 0, cues: new Map(), tbar: null }));
    return d;
  };
  const previewOf = (id: string) => {
    const d = findScreen(id);
    const diff = diffs.get(id);
    const view = screenView(d);
    return {
      screenId: id,
      rev: diff?.rev ?? 0,
      tbar: diff?.tbar ?? null,
      cues: [...(diff?.cues ?? new Map<string, Cue>())].map(([layerId, c]) => ({
        layerId,
        visible: c.visible,
        staged: c.fields !== undefined,
        programChanged: c.programChanged,
      })),
      layers: view.layers.map((l) => {
        const c = diff?.cues.get(l.id);
        return c ? { ...l, onAir: c.visible ?? l.onAir, fields: { ...l.fields, ...c.fields }, pending: false } : l;
      }),
    };
  };
  /** The Preview as a fixture screen ("pv" + id.slice(2)): staged fields, cued visibility. */
  const mirrorPreview = async (d: SDoc) => {
    const diff = diffs.get(d.id);
    const pid = `pv${d.id.slice(2)}`;
    const pd = {
      ...d,
      id: pid,
      layers: d.layers.map((l) => {
        const c = diff?.cues.get(l.id);
        return c?.fields ? { ...l, fields: { ...l.fields, ...c.fields } } : l;
      }),
    };
    await fixture(`savescreen ${JSON.stringify(toFixture(pd))}`);
    for (const l of d.layers) {
      const c = diff?.cues.get(l.id);
      const on = c?.visible ?? onAir.has(`${d.id}/${l.id}`);
      await fixture(`${on ? "layershow" : "layerhide"} ${pid} ${l.id}`);
    }
  };
  /** Take: the diff onto Program, then Preview after take (keep: empty; clear: everything cued out). */
  const takeDiff = async (id: string, diff: Diff, after: boolean) => {
    const d = findScreen(id);
    let doc = d;
    for (const [lid, c] of diff.cues) {
      const key = `${id}/${lid}`;
      if (c.fields) {
        doc = { ...doc, layers: doc.layers.map((l) => (l.id === lid ? { ...l, fields: { ...l.fields, ...c.fields } } : l)) };
        await fixture(`setfields ${id} ${lid} ${JSON.stringify(c.fields)}`);
      }
      if (c.visible === true) onAir.add(key);
      if (c.visible === false) onAir.delete(key);
      if (c.visible !== null) await fixture(`${c.visible ? "layershow" : "layerhide"} ${id} ${lid}`);
    }
    sdocs.set(id, doc);
    diff.cues.clear();
    diff.tbar = null;
    const st = (ui.control as { studio?: { previewAfterTake: string } }).studio;
    if (after && st?.previewAfterTake === "clear")
      for (const l of doc.layers) if (onAir.has(`${id}/${l.id}`)) diff.cues.set(l.id, { visible: false, programChanged: false });
    emit("live-changed", screenView(doc));
  };
  const screenHandlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    list_screen_docs: () => [...sdocs.values()].map(screenSummary),
    get_screen: (a) => screenState(findScreen(a.id)),
    create_screen: async (a) => {
      const n = a.screen as { name: string; width: number; height: number; background: string };
      const d: SDoc = { id: newId(), ...n, layers: [], updatedAt: new Date().toISOString() };
      await putScreen(d);
      return screenState(d);
    },
    save_screen: async (a) => {
      const d = { ...(a.doc as SDoc), updatedAt: new Date().toISOString() };
      findScreen(d.id);
      await putScreen(d);
      return screenState(d);
    },
    rename_screen: async (a) => {
      const d = { ...findScreen(a.id), name: String(a.name), updatedAt: new Date().toISOString() };
      await putScreen(d);
      return screenSummary(d);
    },
    duplicate_screen: async (a) => {
      const d = { ...findScreen(a.id), id: newId(), name: String(a.name), updatedAt: new Date().toISOString() };
      await putScreen(d);
      return screenState(d);
    },
    delete_screen: (a) => {
      findScreen(a.id);
      sdocs.delete(String(a.id));
      return null;
    },
    set_layer_rect: async (a) => {
      const d = findScreen(a.screenId);
      const l = d.layers.find((x) => x.id === a.layerId);
      if (!l) throw { code: "notFound", message: "Layer not found" };
      await fixture(`rect ${d.id} ${l.id} ${JSON.stringify(a.rect)}`);
      return layerView(d, l);
    },
    // Saved to the screen; on air under Live edit "onTake" it is staged ("Changes on next take").
    set_layer_variant: async (a) => {
      const d = findScreen(a.screenId);
      if (!d.layers.some((x) => x.id === a.layerId)) throw { code: "notFound", message: "Layer not found" };
      const next = {
        ...d,
        layers: d.layers.map((l) => (l.id === a.layerId ? { ...l, variant: String(a.variant) } : l)),
        updatedAt: new Date().toISOString(),
      };
      sdocs.set(d.id, next);
      const key = `${d.id}/${String(a.layerId)}`;
      if (onAir.has(key) && ui.control.liveEdit === "onTake") pending.add(key);
      const st = screenState(next);
      emit("live-changed", st.live);
      return st;
    },
    live_show: (a) => setOnAir(a, true),
    live_hide: (a) => setOnAir(a, false),
    set_layer_fields: async (a) => {
      const d = findScreen(a.screenId);
      const fields = a.fields as Record<string, string>;
      if (studioOn()) {
        // Studio mode: edits go to Preview.
        const diff = diffOf(d.id);
        const lid = String(a.layerId);
        const c = diff.cues.get(lid) ?? { visible: null, programChanged: false };
        c.fields = { ...c.fields, ...fields };
        diff.cues.set(lid, c);
        diff.rev++;
        await mirrorPreview(d);
        emit("preview-changed", previewOf(d.id));
        const l = d.layers.find((x) => x.id === lid);
        if (!l) throw { code: "notFound", message: "Layer not found" };
        return layerView(d, l);
      }
      const layers = d.layers.map((l) => (l.id === a.layerId ? { ...l, fields: { ...l.fields, ...fields } } : l));
      const next = { ...d, layers };
      sdocs.set(d.id, next);
      await fixture(`setfields ${d.id} ${String(a.layerId)} ${JSON.stringify(fields)}`);
      const l = layers.find((x) => x.id === a.layerId);
      if (!l) throw { code: "notFound", message: "Layer not found" };
      return layerView(next, l);
    },
    live_clear: async (a) => {
      const d = findScreen(a.screenId);
      for (const l of d.layers) onAir.delete(`${d.id}/${l.id}`);
      await fixture(`clearscreen ${d.id}`);
      return screenView(d);
    },
    live_clear_all: async () => {
      onAir.clear();
      await fixture("clearall");
      return [...sdocs.values()].map(screenView);
    },
    // ---- Studio mode: Preview = Program + a per-screen diff (as core/live.rs keeps it) ----
    set_studio: async (a) => {
      const next = a.settings as { enabled: boolean };
      const exit = a.exit as string | null;
      const was = (ui.control as { studio?: { enabled: boolean } }).studio?.enabled ?? false;
      if (was && !next.enabled) {
        const busy = [...diffs.values()].some((d) => d.cues.size > 0);
        if (busy && !exit) throw { code: "invalid", message: "Preview has changes" };
        for (const [id, d] of diffs) {
          if (exit === "take") await takeDiff(id, d, false);
          else if (exit === "keepStaged")
            // Staged edits of on-air layers wait for the next take; those of off-air layers apply.
            for (const [l, c] of d.cues) {
              if (!c.fields) continue;
              if (onAir.has(`${id}/${l}`)) pending.add(`${id}/${l}`);
              else await takeDiff(id, { rev: 0, tbar: null, cues: new Map([[l, { ...c, visible: null }]]) }, false);
            }
        }
        diffs.clear();
      }
      ui = { ...ui, control: { ...ui.control, studio: a.settings } } as typeof ui;
      return null;
    },
    preview_view: async (a) => {
      const id = String(a.screenId);
      // The fixture's Preview screen is created on first request.
      if (!mirrored.has(id)) {
        mirrored.add(id);
        await mirrorPreview(findScreen(id));
      }
      return previewOf(id);
    },
    studio_op: async (a) => {
      if (!studioOn()) throw { code: "invalid", message: "Studio mode is off" };
      const id = String(a.screenId);
      const d = findScreen(id);
      const op = a.op as { op: string; layerId?: string; visible?: boolean | null; p?: number; commit?: boolean };
      const diff = diffOf(id);
      if (op.op === "cue" && op.layerId) {
        const c = diff.cues.get(op.layerId) ?? { visible: null, programChanged: false };
        c.visible = op.visible ?? null;
        if (c.visible === null && !c.fields) diff.cues.delete(op.layerId);
        else diff.cues.set(op.layerId, c);
      } else if (op.op === "take") await takeDiff(id, diff, true);
      else if (op.op === "tbarBegin") diff.tbar = 0;
      else if (op.op === "tbarSet") diff.tbar = op.p ?? 0;
      else if (op.op === "tbarEnd") {
        diff.tbar = null;
        if (op.commit) await takeDiff(id, diff, true);
      } else if (op.op === "clear") diff.cues.clear();
      // The T-bar position isn't part of the diff (no rev bump, nothing to mirror).
      if (op.op !== "tbarSet" && op.op !== "tbarBegin") {
        diff.rev++;
        await mirrorPreview(d);
      }
      const v = previewOf(id);
      emit("preview-changed", v);
      return v;
    },
    live_panic: async () => {
      diffs.clear();
      onAir.clear();
      await fixture("panic");
      return [...sdocs.values()].map(screenView);
    },
    list_sources: async () => {
      const text = [...srcs.values()].map(sourceSummary);
      const schedules = await Promise.all([...scheds.values()].map(scheduleSummary));
      return [...text, ...schedules, ...[...pollDocs.values()].map(pollSummary)].sort((x, y) => x.name.localeCompare(y.name));
    },
    get_source: (a) => pollDocs.get(String(a.id)) ?? scheds.get(String(a.id)) ?? findSource(a.id),
    // ---- Live polls ----
    polls_quota: () => (s.pollQuota === undefined ? null : s.pollQuota),
    list_polls: () => s.polls ?? [],
    watch_poll: () => null,
    poll_qr: (a) => fakeQr(String(a.text)),
    poll_source_for: () => null,
    project_uses_real_polls: () => [...pollDocs.values()].some((d) => !!d.link),
    simulate_test_poll: () => null,
    reset_test_poll: () => null,
    set_test_poll_status: () => null,
    create_source: (a) => {
      const d: SourceDoc = {
        id: newId(),
        name: String(a.name),
        kind: "text",
        columns: [{ key: "text", label: "Text" }],
        rows: [],
        index: 0,
        updatedAt: new Date().toISOString(),
      };
      srcs.set(d.id, d);
      return d;
    },
    create_schedule: (a) => {
      const d: ScheduleDoc = {
        id: newId(),
        name: String(a.name),
        kind: "schedule",
        items: [],
        ripple: false,
        nextOverride: null,
        labels: { now: "NOW", next: "UP NEXT", delayed: "DELAYED", previous: "PREVIOUS", cancelled: "CANCELLED", upcoming: "" },
        updatedAt: new Date().toISOString(),
      };
      scheds.set(d.id, d);
      return d;
    },
    save_source: (a) => {
      const doc = a.doc as SourceDoc | ScheduleDoc;
      if (doc.kind === "schedule") {
        const d = { ...doc, updatedAt: new Date().toISOString() };
        findSchedule(d.id);
        scheds.set(d.id, d);
        return d;
      }
      const d = { ...doc, updatedAt: new Date().toISOString() };
      findSource(d.id);
      srcs.set(d.id, d);
      return d;
    },
    rename_source: async (a) => {
      const sch = scheds.get(String(a.id));
      if (sch) {
        const d = { ...sch, name: String(a.name) };
        scheds.set(d.id, d);
        return scheduleSummary(d);
      }
      const d = { ...findSource(a.id), name: String(a.name) };
      srcs.set(d.id, d);
      return sourceSummary(d);
    },
    duplicate_source: (a) => {
      const sch = scheds.get(String(a.id));
      if (sch) {
        const d = { ...sch, id: newId(), name: String(a.name) };
        scheds.set(d.id, d);
        return d;
      }
      const d = { ...findSource(a.id), id: newId(), name: String(a.name) };
      srcs.set(d.id, d);
      return d;
    },
    delete_source: (a) => {
      if (scheds.delete(String(a.id))) return null;
      findSource(a.id);
      srcs.delete(String(a.id));
      return null;
    },
    schedule_view: (a) => fetchView(String(a.sourceId)),
    schedule_action: async (a) => {
      const id = String(a.sourceId);
      findSchedule(id);
      await fixture(`scheduleaction ${smap[id] ?? id} ${JSON.stringify(a.action)}`);
      return fetchView(id);
    },
    list_recipes: () => RECIPES,
    list_recipe_instances: () => instances,
    install_recipe: (a) => {
      const req = a.request as { recipeId: string; name: string; screen: { kind: string; screenId?: string } };
      const info = RECIPES.find((r) => r.id === req.recipeId);
      if (!info) throw { code: "notFound", message: "Recipe not found" };
      const inst: RecipeInstance = {
        id: newId(),
        recipeId: info.id,
        name: req.name,
        panel: info.panel,
        sourceId: null,
        screenId: req.screen.screenId ?? "",
        layerIds: [],
        templateIds: [],
        flowIds: [],
        createdAt: new Date().toISOString(),
      };
      instances = [...instances, inst];
      return inst;
    },
    remove_recipe: (a) => {
      const r = findInstance(a.id);
      instances = instances.filter((x) => x.id !== r.id);
      // Its flows go with it.
      flows = flows.filter((f) => f.origin !== r.id && !r.flowIds.includes(f.id));
      return null;
    },
    relink_recipe_source: (a) => {
      const r = findInstance(a.id);
      const id = String(a.sourceId);
      const kind = srcs.has(id) ? "text" : scheds.has(id) ? "schedule" : null;
      if (!kind) throw { code: "notFound", message: "Source not found" };
      const want = r.panel === "slideshow" ? "text" : "schedule";
      if (kind !== want) throw { code: "invalid", message: `This recipe needs a ${want} source` };
      const old = r.sourceId;
      const next = { ...r, sourceId: id };
      instances = instances.map((x) => (x.id === r.id ? next : x));
      // Its flows follow: the old source replaced, its "Source no longer exists" warnings gone.
      flows = flows.map((f) =>
        f.origin === r.id || r.flowIds.includes(f.id)
          ? {
              ...f,
              nodes: f.nodes.map((n) =>
                "sourceId" in n.config && n.config.sourceId === old ? ({ ...n, config: { ...n.config, sourceId: id } } as typeof n) : n,
              ),
              warnings: f.warnings.filter((w) => w.message !== "Source no longer exists"),
            }
          : f,
      );
      return next;
    },
    list_assets: () => assets,
    import_media: () => {
      const job = "job1";
      for (const e of s.importEvents ?? [])
        setTimeout(() => {
          if (e.asset) assets = [...assets.filter((x) => x.name !== e.asset!.name), e.asset];
          emit("asset-import", { job, ...e.progress });
        }, e.afterMs);
      return job;
    },
    cancel_import: () => null,
    "plugin:dialog|open": () => s.dialogPick ?? null,
    media_control: (a) => {
      const key = `${a.screenId}/${a.layerId}`;
      const cur = mediaLive[key];
      if (!cur) throw { code: "notFound", message: "Layer not found" };
      const now = Date.now();
      const act = String(a.action);
      const state = act === "pause" ? "paused" : act === "stop" ? "stopped" : "playing";
      mediaLive[key] = { ...cur, playback: { state, at: now + (state === "playing" ? 120 : 0), posMs: act === "restart" || act === "stop" ? 0 : cur.playback.posMs } };
      const d = findScreen(a.screenId);
      emit("live-changed", screenView(d));
      return null;
    },
    import_assets: () => [],
    delete_asset: (a) => {
      assets = assets.filter((x) => x.name !== a.name);
      return null;
    },
    rename_asset: (a) => {
      const cur = assets.find((x) => x.name === a.name);
      if (!cur) throw { code: "notFound", message: "Image not found" };
      const next = { ...cur, name: String(a.newName), url: `/assets/${encodeURIComponent(String(a.newName))}` };
      assets = assets.map((x) => (x === cur ? next : x));
      return next;
    },
    set_auto_advance: (a) => {
      const d = { ...findSource(a.sourceId), auto: (a.auto as SourceDoc["auto"]) ?? undefined };
      srcs.set(d.id, d);
      return sourceSummary(d);
    },
    set_timer: async (a) => {
      const d = findScreen(a.screenId);
      const key = String(a.key);
      const l = d.layers.find((x) => x.id === a.layerId);
      if (!l) throw { code: "notFound", message: "Layer not found" };
      type T = { run: boolean; at: number; base: number; dir: "up" | "down"; end: "stop" | "hide" };
      const cur: T = { run: false, at: 0, base: 0, dir: "up", end: "stop", ...(JSON.parse(l.fields[key] || "{}") as Partial<T>) };
      const now = Date.now();
      const shown = cur.run ? (cur.dir === "down" ? Math.max(0, cur.base - (now - cur.at)) : cur.base + (now - cur.at)) : cur.base;
      const act = a.action as { action: string; ms?: number; dir?: T["dir"]; end?: T["end"] };
      const next: T =
        act.action === "start"
          ? { ...cur, run: true, at: now, base: shown }
          : act.action === "pause"
            ? { ...cur, run: false, base: shown }
            : act.action === "set"
              ? { ...cur, run: false, base: act.ms ?? 0 }
              : act.action === "configure"
                ? { ...cur, dir: act.dir ?? cur.dir, end: act.end ?? cur.end }
                : { ...cur, run: false, base: 0 };
      const layers = d.layers.map((x) => (x.id === l.id ? { ...x, fields: { ...x.fields, [key]: JSON.stringify(next) } } : x));
      const nd = { ...d, layers };
      sdocs.set(d.id, nd);
      await fixture(`settimer ${d.id} ${l.id} ${key} ${JSON.stringify(a.action)}`);
      return layerView(nd, layers.find((x) => x.id === l.id)!);
    },
    step_source: async (a) => {
      const d = findSource(a.id);
      const step = a.step as { to: "by"; delta: number } | { to: "index"; index: number };
      const max = Math.max(0, d.rows.length - 1);
      const index = Math.min(max, Math.max(0, step.to === "by" ? d.index + step.delta : step.index));
      const delta = index - d.index;
      const next = { ...d, index };
      srcs.set(d.id, next);
      if (delta !== 0) await fixture(`stepsource ${smap[d.id] ?? d.id} ${delta}`);
      return sourceSummary(next);
    },
    list_screens: () => [testScreen, ...[...sdocs.values()].map(screenView)],
    live_screen: (a) => screenView(findScreen(a.screenId)),
  };

  // ---- Events and flows ---------------------------------------------------------------------
  // `listen` keeps the handler's callback id per event, so the mock can emit (e.g. `flow-state`).
  const callbacks = new Map<number, unknown>();
  const listeners = new Map<string, Set<number>>();
  /** Listener (event) id → handler callback id. */
  const listenerIds = new Map<number, number>();
  let nextListener = 1;
  const emit = (event: string, payload: unknown) => {
    for (const h of listeners.get(event) ?? []) {
      const cb = callbacks.get(h);
      if (typeof cb === "function") cb({ event, id: h, payload });
    }
  };
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const flowState = (id: string, state: string) => ({ id, state, node: null, message: null, at: new Date().toISOString() });
  const flowHandlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    list_flows: () => flows,
    get_flow: (a) => findFlow(a.id),
    create_flow: (a) => {
      const f = a.flow as Omit<FlowInfo, "id" | "origin" | "lastFiredAt" | "updatedAt" | "warnings" | "running" | "readOnly"> & {
        origin?: string | null;
      };
      return putFlow({
        ...f,
        id: newId(),
        origin: f.origin ?? null,
        lastFiredAt: null,
        updatedAt: new Date().toISOString(),
        warnings: [],
        running: false,
        readOnly: false,
      });
    },
    save_flow: (a) => {
      const f = a.flow as Pick<FlowInfo, "id" | "name" | "retrigger" | "nodes" | "edges">;
      return putFlow({ ...findFlow(f.id), ...f, draft: null, updatedAt: new Date().toISOString() });
    },
    rename_flow: (a) => putFlow({ ...findFlow(a.id), name: String(a.name) }),
    duplicate_flow: (a) => {
      const f = findFlow(a.id);
      return putFlow({ ...f, id: newId(), name: `${f.name} copy`, origin: null, running: false, lastFiredAt: null });
    },
    delete_flow: (a) => {
      findFlow(a.id);
      flows = flows.filter((x) => x.id !== a.id);
      return null;
    },
    set_flow_armed: (a) => putFlow({ ...findFlow(a.id), armed: Boolean(a.armed), ...(a.armed ? {} : { running: false }) }),
    run_flow: (a) => {
      const id = String(a.id);
      const f = findFlow(id);
      const at = new Date().toISOString();
      putFlow({ ...f, running: true, lastFiredAt: at });
      clearTimeout(timers.get(id));
      setTimeout(() => emit("flow-state", { ...flowState(id, "running"), at }), 0);
      setTimeout(() => emitDebug(findFlow(id)), 0);
      // A cue "runs" for 2.5 s in the mock, then finishes.
      timers.set(
        id,
        setTimeout(() => {
          putFlow({ ...findFlow(id), running: false });
          emit("flow-state", flowState(id, "finished"));
        }, 2500),
      );
      return null;
    },
    stop_flow: (a) => {
      const id = String(a.id);
      clearTimeout(timers.get(id));
      putFlow({ ...findFlow(id), running: false });
      setTimeout(() => emit("flow-state", flowState(id, "stopped")), 0);
      return null;
    },
    // The node editor's draft, deploy, validation and live debug.
    save_flow_draft: (a) => {
      const f = findFlow(a.id);
      const nodes = a.nodes as FlowInfo["nodes"];
      const edges = a.edges as FlowInfo["edges"];
      const same = JSON.stringify({ nodes, edges }) === JSON.stringify({ nodes: f.nodes, edges: f.edges });
      return putFlow({ ...f, draft: same ? null : { nodes, edges, updatedAt: new Date().toISOString() } });
    },
    deploy_flow: (a) => {
      const f = findFlow(a.id);
      if (!f.draft) return f;
      return putFlow({ ...f, nodes: f.draft.nodes, edges: f.draft.edges, draft: null, updatedAt: new Date().toISOString() });
    },
    discard_flow_draft: (a) => putFlow({ ...findFlow(a.id), draft: null }),
    // A small stand-in for the server's check: a node aimed at nothing is an error, a target
    // that doesn't exist a warning, an unwired Branch condition a warning.
    check_flow: (a) => {
      const nodes = a.nodes as FlowInfo["nodes"];
      const edges = a.edges as FlowInfo["edges"];
      const errors: { node: string | null; message: string }[] = [];
      const warnings: { node: string | null; message: string }[] = [];
      if (!nodes.some((n) => n.type.startsWith("trigger."))) errors.push({ node: null, message: "A flow needs a trigger to run" });
      for (const n of nodes) {
        const c = n.config as Record<string, unknown>;
        for (const [k, what] of [["screenId", "screen"], ["layerId", "layer"], ["sourceId", "source"], ["flowId", "flow"]] as const) {
          if (c[k] === "") errors.push({ node: n.id, message: `Pick a ${what}` });
        }
        if (typeof c.screenId === "string" && c.screenId && !sdocs.has(c.screenId) && s.screenDocs) {
          warnings.push({ node: n.id, message: "This screen no longer exists; skipped" });
        }
        if (n.type === "logic.branch" && !edges.some((e) => e.to.node === n.id && e.to.port === "condition")) {
          warnings.push({ node: n.id, message: "The condition is not wired, so it is false" });
        }
      }
      return { errors, warnings };
    },
    list_flow_templates: () => s.flowTemplates ?? [],
    add_flow_template: (a) => {
      const req = a.req as { templateId: string; name?: string | null; slots: Record<string, string> };
      const t = (s.flowTemplates ?? []).find((x) => x.id === req.templateId);
      if (!t) throw { code: "notFound", message: "Flow template not found" };
      const gap = t.slots.find((x) => !x.optional && !req.slots[x.key]);
      if (gap) throw { code: "invalid", message: `${gap.label}: pick a ${gap.kind === "layer" ? "layer" : "value"}` };
      const base = req.name?.trim() || t.name;
      const layerSlot = t.slots.find((x) => x.kind === "layer" && req.slots[x.key]);
      const [screenId = "", layerId = ""] = (layerSlot ? req.slots[layerSlot.key] : "").split("/");
      // As presets/flows: suffixes per flow; armed exactly when the flow has no automatic trigger.
      const suffixes = t.id === "manual-hold" ? ["take", "hold", "release"] : ["start", "end"];
      const armedAt: boolean[] =
        t.id === "break-countdown"
          ? [true, false]
          : t.id === "manual-hold"
            ? [false, true, true]
            : Array(t.flows).fill(["show-open", "next-guest", "stinger-once", "random-fact", "clean-all-log", "reusable-lower-third"].includes(t.id));
      return Array.from({ length: t.flows }, (_, i) => {
        const id = newId();
        const cue = armedAt[i] ?? false;
        const trigger = cue
          ? { id: `${id}t`, type: "trigger.cue", config: { label: null }, position: { x: 0, y: 0 } }
          : { id: `${id}t`, type: "trigger.layer", config: { screenId, layerId, onAir: false }, position: { x: 0, y: 0 } };
        const take = { id: `${id}a`, type: "action.take", config: { screenId, layerId, forMs: null }, position: { x: 240, y: 0 } };
        return putFlow({
          id,
          name: t.flows > 1 ? `${base} · ${suffixes[i]}` : base,
          armed: cue,
          retrigger: "restart",
          origin: null,
          nodes: [trigger, take] as FlowInfo["nodes"],
          edges: [{ id: `${id}w`, from: { node: trigger.id, port: "fired" }, to: { node: take.id, port: "in" }, kind: "event" }],
          draft: null,
          lastFiredAt: null,
          updatedAt: new Date().toISOString(),
          warnings: [],
          running: false,
          readOnly: false,
        });
      });
    },
    subscribe_flow_debug: (a) => {
      debugSubs.set(String(a.id), (debugSubs.get(String(a.id)) ?? 0) + 1);
      return null;
    },
    unsubscribe_flow_debug: (a) => {
      const n = (debugSubs.get(String(a.id)) ?? 0) - 1;
      if (n > 0) debugSubs.set(String(a.id), n);
      else debugSubs.delete(String(a.id));
      return null;
    },
  };
  /** Flow id → open editors (subscribe_flow_debug count). */
  const debugSubs = new Map<string, number>();
  /** A run of a subscribed flow: every node and event wire reachable from its triggers flashes. */
  const emitDebug = (f: FlowInfo) => {
    if (!debugSubs.has(f.id)) return;
    const at = new Date().toISOString();
    const seen = new Set(f.nodes.filter((n) => n.type.startsWith("trigger.")).map((n) => n.id));
    const wires: string[] = [];
    for (let grew = true; grew; ) {
      grew = false;
      for (const e of f.edges) {
        if (e.kind !== "event" || !seen.has(e.from.node) || wires.includes(e.id)) continue;
        wires.push(e.id);
        if (!seen.has(e.to.node)) seen.add(e.to.node);
        grew = true;
      }
    }
    emit("flow-debug", { id: f.id, nodes: [...seen].map((node) => ({ node, at })), edges: wires, errors: [] });
  };
  (window as unknown as { __kinetraFlowDebug: (id: string) => void }).__kinetraFlowDebug = (id) => emitDebug(findFlow(id));
  /** Emits any backend event (e.g. a `flow-debug` batch with a warning, a `flow-state` note). */
  (window as unknown as { __kinetraEmit: (event: string, payload: unknown) => void }).__kinetraEmit = emit;


  // ---- Style library ---------------------------------------------------------------------------
  // apply_style: a layer switches when its template has the look, or was made from a preset that
  // ships it (the look is then added to the template: a copy of its first variant here). On air
  // under Live edit "onTake" it is staged. One undo step per apply.
  const undos = new Map<string, { screenId: string; layerId: string; variant: string | null | undefined }[]>();
  const styleHandlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    list_styles: () => s.styles ?? [],
    apply_style: (a) => {
      const style = String(a.style);
      const scope = a.scope as { kind: "project" } | { kind: "layer"; screenId: string; layerId: string };
      const r: ApplyStyleResult = { switched: 0, unchanged: 0, unchangedTemplates: 0, added: 0, undo: null };
      const custom = new Set<string>();
      const prev: { screenId: string; layerId: string; variant: string | null | undefined }[] = [];
      for (const d of [...sdocs.values()]) {
        if (scope.kind === "layer" && d.id !== scope.screenId) continue;
        let changed = false;
        const layers = d.layers.map((l) => {
          if (scope.kind === "layer" && l.id !== scope.layerId) return l;
          const t = docs.get(l.templateId);
          if (!t) return l;
          const has = t.variants.some((v) => v.id === style);
          const ships = !!t.preset && presets.some((p) => p.id === t.preset?.id && p.variants?.some((v) => v.id === style));
          if (!has && !ships) {
            r.unchanged++;
            custom.add(t.id);
            return l;
          }
          if (!has) {
            const name = s.styles?.find((x) => x.id === style)?.name ?? style;
            docs.set(t.id, { ...t, variants: [...t.variants, { ...t.variants[0], id: style, name }] });
            r.added++;
          }
          if (variantShown(l) === style) return l;
          prev.push({ screenId: d.id, layerId: l.id, variant: l.variant });
          const key = `${d.id}/${l.id}`;
          if (onAir.has(key) && ui.control.liveEdit === "onTake") pending.add(key);
          r.switched++;
          changed = true;
          return { ...l, variant: style };
        });
        if (changed) {
          const next = { ...d, layers, updatedAt: new Date().toISOString() };
          sdocs.set(d.id, next);
          emit("live-changed", screenView(next));
        }
      }
      r.unchangedTemplates = custom.size;
      if (scope.kind === "project") project.settings.style = style;
      if (prev.length) {
        r.undo = newId();
        undos.set(r.undo, prev);
      }
      return r;
    },
    undo_apply_style: (a) => {
      const prev = undos.get(String(a.token));
      if (!prev) throw { code: "notFound", message: "Nothing to undo" };
      undos.delete(String(a.token));
      for (const p of prev) {
        const d = sdocs.get(p.screenId);
        if (!d) continue;
        const next = { ...d, layers: d.layers.map((l) => (l.id === p.layerId ? { ...l, variant: p.variant } : l)) };
        sdocs.set(d.id, next);
        emit("live-changed", screenView(next));
      }
      return null;
    },
    // Validated like variable values: colours #rrggbb / #rrggbbaa / transparent; unset keys are dropped.
    save_brand_kit: (a) => {
      const brand = a.brand as BrandKit;
      const out: BrandKit = {};
      for (const [k, v] of Object.entries(brand) as [keyof BrandKit, string | null | undefined][]) {
        if (!v) continue;
        if (["accent", "secondary", "text", "plate"].includes(k) && !/^(#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?|transparent)$/i.test(v))
          throw { code: "invalid", message: `Brand ${k}: “${v}” is not a color; use #rgb, #rrggbb, #rrggbbaa or transparent` };
        out[k] = /^transparent$/i.test(v) ? "transparent" : v;
      }
      project.settings.brand = out;
      return out;
    },
  };

  // ---- API sources ------------------------------------------------------------------------
  const LEAGUE = {
    competition: { name: "Eliteserien", season: 2026 },
    updated: new Date(Date.now() - 4000).toISOString(),
    standings: [
      ["Bodø/Glimt", "BOD", 61, 26, 41],
      ["Brann", "BRA", 55, 26, 22],
      ["Viking", "VIK", 52, 26, 18],
      ["Molde", "MOL", 47, 26, 12],
      ["Rosenborg", "RBK", 44, 26, 9],
      ["Fredrikstad", "FFK", 39, 26, 1],
    ].map(([name, short, points, played, gd], i) => ({
      position: i + 1,
      team: { name, short, crest: `https://img.example.com/crests/${String(short).toLowerCase()}.png` },
      played,
      goalDifference: gd,
      points,
      form: i === 0 ? "WWDWW" : null,
    })),
  };
  const WEATHER = "City;Temp;Condition\nOslo;12;Cloudy\nBergen;10;Rain\nTrondheim;9;Showers\nTromsø;6;Clear\n";
  const parseCsv = (text: string): string[][] => text.trim().split("\n").map((l) => l.split(";"));
  const findApi = (id: unknown) => {
    const d = findSource(id);
    if (d.kind !== "api") throw { code: "invalid", message: "Not an API source" };
    return d;
  };
  const putApi = (id: string, patch: Partial<ApiStatus>) => {
    const d = findApi(id);
    const st = { ...(apiOf(d) as ApiStatus), ...patch };
    apiStatus.set(id, st);
    const sum = sourceSummary(d);
    emit("source-changed", sum);
    return sum;
  };
  const apiHandlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    create_api_source: (a) => {
      const d: SourceDoc = {
        id: newId(),
        name: String(a.name),
        kind: "api",
        request: { url: "", method: "GET", headers: [], body: null },
        format: "json",
        mapping: { rows: "", columns: [], csv: { header: true, delimiter: "auto" } },
        poll: { intervalMs: 10000, onlyWhenUsed: true, paused: false },
        columns: [],
        rows: [],
        index: 0,
        fetchedAt: null,
        updatedAt: new Date().toISOString(),
      };
      srcs.set(d.id, d);
      apiStatus.set(d.id, { state: "idle", lastOkAt: null, lastAttemptAt: null, nextAt: null, httpStatus: null, message: null, rows: 0 });
      return d;
    },
    test_api_source: async (a) => {
      const d = a.doc as Extract<SourceDoc, { kind: "api" }>;
      await new Promise((r) => setTimeout(r, 150));
      if (d.format === "csv") {
        const [head, ...body] = parseCsv(WEATHER);
        const names = d.mapping.csv.header ? head : head.map((_, i) => String(i));
        const data = d.mapping.csv.header ? body : [head, ...body];
        const rows = data.map((cells) =>
          Object.fromEntries(d.mapping.columns.map((c) => [c.key, cells[names.indexOf(c.path)] ?? ""])),
        );
        return {
          httpStatus: 200,
          headers: [["content-type", "text/csv; charset=utf-8"], ["cache-control", "max-age=60"]],
          preview: WEATHER,
          json: null,
          csvColumns: names,
          rows,
          hits: Object.fromEntries(d.mapping.columns.map((c) => [c.key, rows.filter((r) => r[c.key] !== "").length])),
          error: null,
        };
      }
      return {
        httpStatus: 200,
        headers: [
          ["content-type", "application/json"],
          ["etag", "\"5f2c-1a9\""],
          ["x-ratelimit-remaining", "58"],
          ["authorization", "[redacted]"],
        ],
        preview: JSON.stringify(LEAGUE, null, 2),
        json: LEAGUE,
        csvColumns: null,
        rows: [],
        hits: {},
        error: null,
      };
    },
    set_source_secret: (a) => {
      const d = findApi(a.sourceId);
      const next = {
        ...d,
        request: {
          ...d.request,
          headers: d.request.headers.map((h) =>
            h.name === a.header ? { name: h.name, value: null, secret: h.secret ?? newId(), stored: true } : h,
          ),
        },
        updatedAt: new Date().toISOString(),
      };
      srcs.set(d.id, next);
      return next;
    },
    clear_source_secret: (a) => {
      const d = findApi(a.sourceId);
      const next = {
        ...d,
        request: {
          ...d.request,
          headers: d.request.headers.map((h) => (h.name === a.header ? { name: h.name, value: null, secret: null } : h)),
        },
        updatedAt: new Date().toISOString(),
      };
      srcs.set(d.id, next);
      return next;
    },
    retry_api_source: (a) =>
      putApi(String(a.id), { state: "ok", lastOkAt: Date.now(), lastAttemptAt: Date.now(), message: null, httpStatus: 200 }),
    set_api_paused: (a) => {
      const d = findApi(a.id);
      srcs.set(d.id, { ...d, poll: { ...d.poll, paused: a.paused === true } });
      return putApi(d.id, a.paused === true ? { state: "paused", nextAt: null } : { state: "ok", nextAt: Date.now() + d.poll.intervalMs });
    },
    watch_api_source: () => null,
  };

  // ---- MIDI / OSC control ------------------------------------------------------------------------
  let controlsDoc: ControlsDoc = s.controls ?? { mappings: [] };
  let learnTimer: ReturnType<typeof setTimeout> | undefined;
  const midiHandlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    list_midi_devices: () => s.midiDevices ?? [],
    test_midi_feedback: () => null,
    watch_midi_devices: () => null,
    osc_status: () => s.oscStatus ?? { state: "listening", port: 9000, lan: false },
    get_controls: () => controlsDoc,
    save_controls: (a) => {
      controlsDoc = a.doc as ControlsDoc;
      setTimeout(() => emit("controls-changed", controlsDoc), 0);
      return controlsDoc;
    },
    midi_learn: (a) => {
      clearTimeout(learnTimer);
      const delay = s.midiLearnDelayMs === undefined ? 600 : s.midiLearnDelayMs;
      if (a.on && s.midiLearnNext && delay !== null) learnTimer = setTimeout(() => emit("midi-learn", s.midiLearnNext), delay);
      return null;
    },
    update_project_settings: (a) => {
      project.settings = a.settings as Record<string, unknown>;
      return project;
    },
  };

  // ---- Display outputs ---------------------------------------------------------------------------
  const displays = s.displays ?? [];
  const patterns = new Map<string, string | null>();
  const screenIds = () => new Set(s.screenDocs ? s.screenDocs.map((d) => d.id) : screens.map((x) => x.screen.id));
  const adjacent = (a: DisplayInfo, b: DisplayInfo) =>
    a.refreshHz === b.refreshHz &&
    a.width === b.width &&
    a.height === b.height &&
    ((a.y === b.y && (a.x + a.width === b.x || b.x + b.width === a.x)) || (a.x === b.x && (a.y + a.height === b.y || b.y + b.height === a.y)));
  const outputStatus = (): OutputStatus[] => {
    const configs = (prefs.outputs as OutputConfig[] | undefined) ?? [];
    const shown = screenIds();
    return configs.map((o) => {
      const d = displays.find((x) => x.id === o.display.id) ?? null;
      const pattern = (patterns.get(o.id) ?? null) as OutputStatus["pattern"];
      const fill = o.mode === "key" ? configs.find((c) => c.id === o.keyOf) : undefined;
      const partner = o.mode === "key" ? fill : configs.find((c) => c.keyOf === o.id && c.mode === "key");
      const pd = partner ? displays.find((x) => x.id === partner.display.id) : undefined;
      const screen = o.mode === "key" ? (fill?.screenId ?? null) : o.screenId;
      const state: OutputStatus["state"] =
        !o.enabled && !pattern ? "off" : !d ? "displayMissing" : s.noProject ? "noProject" : !pattern && (!screen || !shown.has(screen)) ? "screenMissing" : "showing";
      return {
        id: o.id,
        state,
        displayId: d?.id ?? null,
        frameLocked: partner && d && pd && state === "showing" ? adjacent(d, pd) : null,
        pattern,
        message: null,
        ...s.outputStatus?.[o.id],
      };
    });
  };
  const outputsChanged = () => setTimeout(() => emit("outputs-changed", outputStatus()), 0);
  const outputHandlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    list_displays: () => displays,
    watch_displays: () => displays,
    list_outputs: () => outputStatus(),
    output_test_pattern: (a) => {
      patterns.set(String(a.id), (a.pattern as string | null) ?? null);
      outputsChanged();
      return null;
    },
    identify_displays: () => null,
    all_outputs_off: () => {
      const configs = (prefs.outputs as OutputConfig[] | undefined) ?? [];
      prefs = { ...prefs, outputs: configs.map((o) => ({ ...o, enabled: false })) };
      patterns.clear();
      outputsChanged();
      return null;
    },
    list_screen_docs: () =>
      screens.filter((x) => x.screen.id !== "test").map((x) => ({ ...x.screen, layerCount: x.layers.length, updatedAt: new Date().toISOString() })),
  };

  const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    ...outputHandlers,
    ...midiHandlers,
    ...flowHandlers,
    ...apiHandlers,
    ...styleHandlers,
    list_templates: () => [...docs.values()].map(summary),
    get_template: (a) => find(a.id),
    list_presets: () => presets,
    create_template: (a) => {
      const source = a.source as { kind: "blank" } | { kind: "preset"; presetId: string };
      const base =
        source.kind === "preset"
          ? [...docs.values()].find((d) => d.preset?.id === source.presetId)
          : undefined;
      const doc: Doc = {
        id: newId(),
        name: String(a.name),
        description: base?.description ?? "",
        preset: base?.preset ?? null,
        fields: base?.fields ?? [],
        variables: base?.variables ?? [],
        autoOutMs: base?.autoOutMs ?? null,
        updatedAt: new Date().toISOString(),
        // A new template is one "Default" look (a preset's first variant).
        defaultVariant: "default",
        variants: [
          {
            id: "default",
            name: "Default",
            vars: {},
            html: base?.variants[0]?.html ?? "<div class=\"root\"></div>\n",
            css: base?.variants[0]?.css ?? "",
          },
        ],
      };
      docs.set(doc.id, doc);
      return doc;
    },
    // The shared parts plus each EXISTING variant's look; the variant list, names and the default
    // stay as on disk (as core/templates.rs does).
    save_template: (a) => {
      const sent = a.doc as Doc;
      const old = find(sent.id);
      const doc: Doc = {
        ...sent,
        defaultVariant: old.defaultVariant,
        variants: old.variants.map((v) => {
          const n = sent.variants?.find((x) => x.id === v.id);
          return n ? { ...v, html: n.html, css: n.css, vars: n.vars } : v;
        }),
        updatedAt: new Date().toISOString(),
      };
      docs.set(doc.id, doc);
      return doc;
    },
    rename_template: (a) => {
      const doc = { ...find(a.id), name: String(a.name), updatedAt: new Date().toISOString() };
      docs.set(doc.id, doc);
      return summary(doc);
    },
    duplicate_template: (a) => {
      const doc = { ...find(a.id), id: newId(), name: String(a.name), updatedAt: new Date().toISOString() };
      docs.set(doc.id, doc);
      return doc;
    },
    delete_template: (a) => {
      find(a.id);
      docs.delete(String(a.id));
      return null;
    },
    // ---- Variants: one field schema, several looks (same rules as core/templates.rs) ----
    save_template_variant: (a) =>
      updateVariants(a.templateId, (d) => ({
        variants: d.variants.map((v) =>
          v.id === a.variantId
            ? { ...v, html: String(a.html), css: String(a.css), vars: a.vars as Record<string, string> }
            : v,
        ),
      })),
    create_template_variant: (a) =>
      updateVariants(a.templateId, (d) => {
        const from = findVariant(d, a.fromVariant);
        if (d.variants.length >= 12) throw { code: "invalid", message: "A template can have at most 12 variants" };
        if (d.variants.some((v) => v.name.toLowerCase() === String(a.name).toLowerCase()))
          throw { code: "invalid", message: `There is already a variant named “${String(a.name)}”` };
        const slug = String(a.name).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "variant";
        let id = /^[a-z]/.test(slug) ? slug.slice(0, 32) : `v-${slug}`.slice(0, 32);
        for (let n = 2; d.variants.some((v) => v.id === id); n++) id = `${slug.slice(0, 28)}-${n}`;
        return { variants: [...d.variants, { ...from, id, name: String(a.name) }] };
      }),
    rename_template_variant: (a) =>
      updateVariants(a.templateId, (d) => {
        findVariant(d, a.variantId);
        return { variants: d.variants.map((v) => (v.id === a.variantId ? { ...v, name: String(a.name) } : v)) };
      }),
    delete_template_variant: (a) =>
      updateVariants(a.templateId, (d) => {
        findVariant(d, a.variantId);
        if (d.variants.length <= 1) throw { code: "invalid", message: "A template needs at least one variant" };
        const variants = d.variants.filter((v) => v.id !== a.variantId);
        return {
          variants,
          defaultVariant: d.defaultVariant === a.variantId ? variants[0].id : d.defaultVariant,
        };
      }),
    set_default_template_variant: (a) =>
      updateVariants(a.templateId, (d) => ({ defaultVariant: findVariant(d, a.variantId).id })),
    preview_template: async (a) => {
      const doc = find(a.id);
      preview.doc = doc;
      preview.onAir = false;
      // As the server previews: labels; lists three rows of "<column label> n"; timers their
      // default; clocks their format.
      preview.values = Object.fromEntries(
        doc.fields.map((f) => [
          f.key,
          f.type === "list"
            ? JSON.stringify([1, 2, 3].map((n) => Object.fromEntries((f.columns ?? []).map((c) => [c.key, `${c.label} ${n}`]))))
            : f.type === "timer"
              ? f.default
              : f.type === "clock"
                ? f.format || "HH:mm:ss"
                : f.label,
        ]),
      );
      await fixture(doc.preset ? `template ${doc.preset.id}` : "clear");
      return previewLayer();
    },
    preview_draft: async (a) => {
      const d = a.draft as { fieldValues?: Record<string, string>; variableValues?: Record<string, string> };
      if (d.fieldValues) {
        preview.values = { ...preview.values, ...d.fieldValues };
        await fixture(`fields ${JSON.stringify(d.fieldValues)}`);
      }
      if (d.variableValues) await fixture(`vars ${JSON.stringify(d.variableValues)}`);
      return previewLayer();
    },
    preview_show: async () => {
      preview.onAir = true;
      await fixture("show");
      return previewLayer();
    },
    preview_hide: async () => {
      preview.onAir = false;
      await fixture("hide");
      return previewLayer();
    },

    get_prefs: () => prefs,
    set_prefs: (a) => {
      prefs = a.prefs as typeof prefs;
      outputsChanged();
      return prefs;
    },
    current_project: () => (s.noProject ? null : project),
    get_ui_state: () => ui,
    set_ui_state: (a) => {
      ui = a.uiState as typeof ui;
      // Clamped like clamp_ui_state (CONTROL_PANEL_WIDTH_RANGE).
      const w = ui.control.panelWidth;
      if (typeof w === "number") ui = { ...ui, control: { ...ui.control, panelWidth: Math.min(640, Math.max(280, w)) } };
      return ui;
    },
    list_recents: () => s.recents ?? [],
    server_status: () => status,
    list_screens: () => screens,
    live_screen: (a) => screens.find((x) => x.screen.id === a.screenId) ?? testScreen,
    list_backups: () => backups,
    app_info: () => ({
      version: "0.1.0",
      tauriVersion: "2.8.5",
      webviewVersion: "141.0.3537.57",
      configDir: "C:\\Users\\me\\AppData\\Roaming\\no.rorvikmedieteknikk.kinetra",
      logDir: "C:\\Users\\me\\AppData\\Local\\no.rorvikmedieteknikk.kinetra\\logs",
      channel: "stable",
      commit: "fa23a08",
    }),
    // ---- Licensing ----
    license_view: () => {
      const now = Math.floor(Date.UTC(2026, 9, 10, 12) / 1000);
      const act = (id: string, name: string, own: boolean, slot = "main") => ({
        id, machine_name: name, slot, created_at: null, last_refreshed_at: null, this_machine: own,
      });
      return {
        signedIn: true,
        account: { email: "ola@example.com", name: "Ola Nordmann" },
        status: "active",
        access: "full",
        plan: "pro_yearly",
        planName: "Kinetra Pro (yearly)",
        entitlements: ["core.app", "pro.ai", "pro.polls"],
        features: { ai: true, polls: true },
        pending: null,
        endsAt: now + 200 * 86400,
        updatesUntil: null,
        fullUntil: now + 18 * 86400,
        reducedUntil: now + 32 * 86400,
        offlineDaysLeft: 18,
        lastCheckAt: now - 180,
        online: true,
        machineName: "EDIT-PC",
        slot: "main",
        licenses: [
          { id: "l1", plan: "pro_yearly", kind: "subscription", status: "active", entitlements: [], seats: 2, seats_used: 2,
            backup_used: true, starts_at: null, ends_at: null, updates_until: null, revoked_at: null, beta_program_id: null,
            activations: [act("a1", "EDIT-PC", true), act("a2", "STUDIO-PC", false), act("a3", "TRUCK-BACKUP", false, "backup")] },
        ],
        upgradeBlocked: null,
        beta: null,
        notice: null,
        welcome: false,
        onAir: false,
        signin: { state: "idle" },
        ...s.license,
      };
    },
    license_list: () => [],
    license_check_now: () => null,
    cloud_links: () => ({ account: "https://account.kinetra.live/account", pricing: "https://kinetra.live/pricing", download: "https://kinetra.live/download" }),
    sign_in_start: () => ({ state: "idle" }),
    // ---- Preferences window, zoom, updates ----
    open_preferences: () => null,
    set_zoom: (a) => {
      const z = Math.round(Math.min(2, Math.max(0.5, Number(a.zoom))) * 10) / 10;
      prefs = { ...prefs, zoom: z === 1 ? undefined : z };
      return z;
    },
    check_for_update: async () => {
      await new Promise((r) => setTimeout(r, 400));
      if (s.update === "fail") throw { code: "updateCheck", message: "Couldn't check for updates. Try again later." };
      return s.update ?? { available: false, version: null, notes: null, date: null, blocked: null };
    },
    install_update: () => {
      // Downloads forever (a screenshot of the progress): 40 % of 18 MB.
      setTimeout(() => emit("update-progress", { downloaded: 7_340_032, total: 18_350_080, phase: "download" }), 300);
      return new Promise(() => {});
    },
    "plugin:event|listen": (a) => {
      const set = listeners.get(String(a.event)) ?? new Set<number>();
      set.add(Number(a.handler));
      listeners.set(String(a.event), set);
      listenerIds.set(nextListener, Number(a.handler));
      return nextListener++;
    },
    "plugin:event|unlisten": (a) => {
      const h = listenerIds.get(Number(a.eventId));
      if (h !== undefined) listeners.get(String(a.event))?.delete(h);
      return null;
    },
    "plugin:window|show": () => null,
    // Project screens replace the old `screens` list when the scenario has them.
    ...(s.screenDocs ? screenHandlers : {}),
  };

  let next = 1;
  w.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
    transformCallback(cb: unknown) {
      const id = next++;
      callbacks.set(id, cb);
      return id;
    },
    unregisterCallback(id: number) {
      callbacks.delete(id);
      for (const set of listeners.values()) set.delete(id);
    },
    async invoke(cmd: string, args: Record<string, unknown> = {}) {
      const h = handlers[cmd];
      return h ? structuredClone(await h(args)) : null;
    },
    convertFileSrc: (p: string) => p,
  };
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
}
