import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, RecentProject } from "$lib/types";

const h = vi.hoisted(() => {
  const listeners = new Map<string, (payload: unknown) => void>();
  return {
    listeners,
    api: {
      currentProject: vi.fn(),
      listRecents: vi.fn(),
      removeRecent: vi.fn(),
      openProject: vi.fn(),
      createProject: vi.fn(),
      initProject: vi.fn(),
      getUiState: vi.fn(),
      setUiState: vi.fn(),
      getPrefs: vi.fn(),
      setPrefs: vi.fn(),
      on: vi.fn((event: string, handler: (payload: unknown) => void) => {
        listeners.set(event, handler);
        return () => listeners.delete(event);
      }),
    },
    flow: {
      pickFolder: vi.fn(),
      openFolder: vi.fn(),
      revealInExplorer: vi.fn(),
    },
    goto: vi.fn(),
    cloud: {
      cloudView: vi.fn(),
      cloudRefresh: vi.fn(),
      cloudProjectsList: vi.fn(),
      cloudRestorePlan: vi.fn(),
      cloudRestoreStart: vi.fn(),
      cloudRestoreCancel: vi.fn(),
      cloudRestoreDismiss: vi.fn(),
    },
  };
});

vi.mock("$lib/api", () => ({
  ...h.api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/projectFlow", () => ({
  ...h.flow,
  describeEntries: (n: number) => (n === 1 ? "1 item" : `${n} items`),
}));
vi.mock("$app/navigation", () => ({ goto: h.goto }));
vi.mock("$lib/api/cloud", () => h.cloud);
vi.mock("$lib/api/cloud-history", () => h.cloud);
vi.mock("$lib/api/cloud-restore", () => h.cloud);

import Page from "./+page.svelte";
import { project } from "$lib/stores/project.svelte";
import { uiState } from "$lib/stores/uiState.svelte";

const proj: Project = {
  path: "C:\\Shows\\friday",
  name: "Friday",
  createdAt: "2026-01-01T00:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } },
};

const hoursAgo = (n: number) => new Date(Date.now() - n * 3600_000).toISOString();

const recents: RecentProject[] = [
  { path: "C:\\Shows\\friday", name: "Friday", lastOpenedAt: hoursAgo(2), missing: false },
  { path: "D:\\Old\\gone", name: "Gone show", lastOpenedAt: hoursAgo(50), missing: false },
];

async function renderPage() {
  const r = render(Page);
  // Let onMount's refresh + loadRecents settle.
  await waitFor(() => expect(h.api.listRecents).toHaveBeenCalled());
  await tick();
  return r;
}

describe("project picker", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    h.listeners.clear();
    history.replaceState(null, "", "/"); // the picker reads ?new from location
    project.current = null;
    project.recents = [];
    project.busy = false;
    uiState.set(null);
    h.api.currentProject.mockResolvedValue(null);
    h.api.listRecents.mockResolvedValue([]);
    h.api.getUiState.mockResolvedValue({
      sidebarPinned: false,
      control: {
        view: "grid",
        gridColumns: 2,
        selectedScreenId: null,
        hiddenScreenIds: [],
        liveEdit: "immediate",
        hotkeys: {},
      },
    });
  });
  afterEach(() => cleanup());

  it("renders the three actions", async () => {
    await renderPage();
    expect(screen.getByRole("button", { name: /new project/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /open project/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /in the cloud/i }).getAttribute("aria-expanded")).toBe("false");
    expect(screen.getByText("Projects you open will appear here.")).toBeTruthy();
  });

  it("downloads a project from the cloud and opens it", async () => {
    const view = (restore: unknown = null) => ({
      access: "available",
      state: "ok",
      offline: false,
      checkedAt: 1,
      quota: null,
      settings: { onAir: "pause", limitKbps: 2000, intervalS: 300, newProjects: true },
      project: null,
      restore,
    });
    h.cloud.cloudView.mockResolvedValue(view());
    h.cloud.cloudProjectsList.mockResolvedValue([
      { id: "p1", name: "Gala", versions: 3, kept: 0, conflict: false, lastAt: "2026-10-15T14:02:00Z",
        lastMachine: "STUDIO-PC", trashedAt: null, purgeAt: null, open: false, here: false },
    ]);
    h.cloud.cloudRestorePlan.mockResolvedValue({ folder: "C:\\Shows\\gala", backupFirst: false, copyFolder: null });
    h.cloud.cloudRestoreStart.mockResolvedValue(view());
    h.cloud.cloudRestoreDismiss.mockResolvedValue(view());
    h.flow.openFolder.mockResolvedValue({ status: "opened", project: proj });
    await renderPage();
    await fireEvent.click(screen.getByRole("button", { name: /in the cloud/i }));
    await fireEvent.click(await screen.findByRole("button", { name: "Download" }));
    expect(await screen.findByText("C:\\Shows\\gala")).toBeTruthy();
    expect(h.cloud.cloudRestorePlan).toHaveBeenCalledWith("download", "Gala", "2026-10-15T14:02:00Z");
    await fireEvent.click(screen.getByRole("button", { name: "Download" }));
    await waitFor(() =>
      expect(h.cloud.cloudRestoreStart).toHaveBeenCalledWith({
        projectId: "p1",
        versionId: null,
        mode: "download",
        folder: "C:\\Shows\\gala",
      }),
    );
    // The core reports the finished restore.
    const result = { ok: true, error: null, damaged: [], damagedCount: 0, copyFolder: null, reused: 0 };
    h.listeners.get("cloud-changed")?.(
      view({ mode: "download", versionId: "v1", folder: "C:\\Shows\\gala", phase: "downloading",
             doneBytes: 1, totalBytes: 1, paused: false, limited: false, result }),
    );
    expect(await screen.findByText("Restored into gala.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Open project" }));
    await waitFor(() => expect(h.flow.openFolder).toHaveBeenCalledWith("C:\\Shows\\gala"));
    await waitFor(() => expect(h.goto).toHaveBeenCalledWith("/dashboard"));
  });

  it("offers a check for backups without Pro", async () => {
    h.cloud.cloudView.mockResolvedValue({ access: "free", project: null, restore: null });
    await renderPage();
    await fireEvent.click(screen.getByRole("button", { name: /in the cloud/i }));
    expect(await screen.findByText("Cloud backup is part of Kinetra Pro.")).toBeTruthy();
    h.cloud.cloudRefresh.mockResolvedValue({ access: "available", project: null, restore: null });
    h.cloud.cloudProjectsList.mockResolvedValue([]);
    await fireEvent.click(screen.getByRole("button", { name: "Check for backups" }));
    expect(await screen.findByText("No projects are backed up to the cloud yet.")).toBeTruthy();
    expect(h.cloud.cloudRefresh).toHaveBeenCalledWith(true);
  });

  it("goes to the dashboard when a project is already open", async () => {
    h.api.currentProject.mockResolvedValue(proj);
    render(Page);
    await waitFor(() => expect(h.goto).toHaveBeenCalledWith("/dashboard", { replaceState: true }));
  });

  it("?new expands the form, focuses Name, and does not redirect", async () => {
    history.replaceState(null, "", "/?new=1");
    h.api.currentProject.mockResolvedValue(proj);
    await renderPage();
    const name = await screen.findByLabelText("Name");
    await waitFor(() => expect(document.activeElement).toBe(name));
    expect(screen.getByRole("button", { name: /new project/i }).getAttribute("aria-expanded")).toBe(
      "true",
    );
    expect(h.goto).not.toHaveBeenCalled();
  });

  it("Open → needsConfirm warns with the entry count and only inits on confirm", async () => {
    h.flow.pickFolder.mockResolvedValue("C:\\Stuff");
    h.flow.openFolder.mockResolvedValue({ status: "needsConfirm", path: "C:\\Stuff", entries: 12 });
    h.api.initProject.mockResolvedValue(proj);
    await renderPage();

    await fireEvent.click(screen.getByRole("button", { name: /open project/i }));
    const heading = await screen.findByText(/isn't empty and isn't a Kinetra project/);
    const panel = heading.closest("section")!;
    expect(within(panel).getByText(/12 items/)).toBeTruthy();
    expect(within(panel).getByText("C:\\Stuff")).toBeTruthy();
    expect(h.api.initProject).not.toHaveBeenCalled();
    expect(h.flow.pickFolder).toHaveBeenCalledWith("Open a project folder");

    await fireEvent.click(within(panel).getByRole("button", { name: /initialize here anyway/i }));
    await waitFor(() => expect(h.api.initProject).toHaveBeenCalledWith("C:\\Stuff", undefined));
    await waitFor(() => expect(h.goto).toHaveBeenCalledWith("/dashboard"));
  });

  it("Cancel on the warning writes nothing", async () => {
    h.flow.pickFolder.mockResolvedValue("C:\\Stuff");
    h.flow.openFolder.mockResolvedValue({ status: "needsConfirm", path: "C:\\Stuff", entries: 1 });
    await renderPage();
    await fireEvent.click(screen.getByRole("button", { name: /open project/i }));
    await screen.findByText(/1 item/);
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText(/isn't empty/)).toBeNull();
    expect(h.api.initProject).not.toHaveBeenCalled();
  });

  it("a cancelled folder dialog is not an error", async () => {
    h.flow.pickFolder.mockResolvedValue(null);
    await renderPage();
    await fireEvent.click(screen.getByRole("button", { name: /open project/i }));
    await tick();
    expect(h.flow.openFolder).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("an AppError from open renders inline with role=alert", async () => {
    h.flow.pickFolder.mockResolvedValue("C:\\New");
    h.flow.openFolder.mockRejectedValue({
      code: "newerVersion",
      message: "This project was saved by a newer version of Kinetra.",
    });
    await renderPage();
    await fireEvent.click(screen.getByRole("button", { name: /open project/i }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/newer version/);
  });

  it("New project: Create needs a name and location, submits the trimmed name", async () => {
    h.flow.pickFolder.mockResolvedValue("C:\\Shows");
    h.api.createProject.mockResolvedValue(proj);
    await renderPage();

    await fireEvent.click(screen.getByRole("button", { name: /new project/i }));
    const create = (await screen.findByRole("button", { name: "Create" })) as HTMLButtonElement;
    expect(create.disabled).toBe(true);

    const name = screen.getByLabelText("Name") as HTMLInputElement;
    expect(document.activeElement).toBe(name);
    await fireEvent.input(name, { target: { value: "  My Show!  " } });
    expect(create.disabled).toBe(true); // still no location

    await fireEvent.click(screen.getByRole("button", { name: "Choose…" }));
    await waitFor(() => expect(create.disabled).toBe(false));
    expect(h.flow.pickFolder).toHaveBeenCalledWith("Choose where to create the project");
    expect(screen.getByText("C:\\Shows\\my-show")).toBeTruthy();

    await fireEvent.submit(name.form!);
    await waitFor(() => expect(h.api.createProject).toHaveBeenCalledWith("C:\\Shows", "My Show!"));
    await waitFor(() => expect(h.goto).toHaveBeenCalledWith("/dashboard"));
  });

  it("New project: Esc collapses the form; Ctrl+N expands it", async () => {
    await renderPage();
    await fireEvent.keyDown(window, { key: "n", ctrlKey: true });
    const name = await screen.findByLabelText("Name");
    expect(document.activeElement).toBe(name);
    await fireEvent.keyDown(name, { key: "Escape" });
    expect(screen.queryByLabelText("Name")).toBeNull();
  });

  it("New project: a notEmpty error shows inline", async () => {
    h.flow.pickFolder.mockResolvedValue("C:\\Shows");
    h.api.createProject.mockRejectedValue({ code: "notEmpty", message: "exists" });
    await renderPage();
    await fireEvent.click(screen.getByRole("button", { name: /new project/i }));
    await fireEvent.input(await screen.findByLabelText("Name"), { target: { value: "Show" } });
    await fireEvent.click(screen.getByRole("button", { name: "Choose…" }));
    const create = screen.getByRole("button", { name: "Create" }) as HTMLButtonElement;
    await waitFor(() => expect(create.disabled).toBe(false));
    await fireEvent.click(create);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/already exists/);
    expect(h.goto).not.toHaveBeenCalled();
  });

  it("recents render; a recents-checked update marks one missing and non-openable", async () => {
    h.api.listRecents.mockResolvedValue(recents);
    h.api.openProject.mockResolvedValue(proj);
    await renderPage();

    const list = screen.getByRole("list", { name: "Recent projects" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(2);
    expect(within(list).getByText("2 hours ago")).toBeTruthy();
    expect(within(list).getByRole("button", { name: /gone show/i })).toBeTruthy();
    expect(screen.queryByText("Missing")).toBeNull();

    h.listeners.get("recents-checked")!([recents[0], { ...recents[1], missing: true }]);
    await tick();

    expect(screen.getByText("Missing")).toBeTruthy();
    expect(within(list).queryByRole("button", { name: /gone show/i })).toBeNull();
    expect(within(list).getByRole("button", { name: "Remove" })).toBeTruthy();

    await fireEvent.click(within(list).getByRole("button", { name: /friday/i }));
    await waitFor(() => expect(h.api.openProject).toHaveBeenCalledWith("C:\\Shows\\friday"));
    await waitFor(() => expect(h.goto).toHaveBeenCalledWith("/dashboard"));
  });

  it("recent row actions reveal and remove", async () => {
    h.api.listRecents.mockResolvedValue(recents);
    h.api.removeRecent.mockResolvedValue([recents[1]]);
    h.flow.revealInExplorer.mockResolvedValue(undefined);
    await renderPage();
    const list = screen.getByRole("list", { name: "Recent projects" });
    const [first] = within(list).getAllByRole("listitem");
    await fireEvent.click(within(first).getByRole("button", { name: "Show in Explorer" }));
    expect(h.flow.revealInExplorer).toHaveBeenCalledWith("C:\\Shows\\friday");
    await fireEvent.click(within(first).getByRole("button", { name: "Remove from list" }));
    await waitFor(() => expect(within(list).getAllByRole("listitem")).toHaveLength(1));
    expect(h.api.removeRecent).toHaveBeenCalledWith("C:\\Shows\\friday");
  });

  it("shows the Kinetra logo as the page heading", async () => {
    await renderPage();
    const h1 = screen.getByRole("heading", { level: 1, name: "Kinetra" });
    const imgs = [...h1.querySelectorAll("img")];
    expect(imgs.map((i) => i.className.split(" ")[0])).toEqual(["for-dark", "for-light"]);
    expect(imgs.every((i) => i.getAttribute("height") === "56")).toBe(true);
    // No channel badge on the picker, in any build.
    expect(h1.textContent?.toLowerCase()).not.toContain("canary");
    expect(screen.getByText("Open a project to start.")).toBeTruthy();
  });

  it("action panels draw viewfinder brackets; the expanded one is selected", async () => {
    await renderPage();
    const newCard = screen.getByRole("button", { name: /new project/i });
    const openCard = screen.getByRole("button", { name: /open project/i });
    const cloud = screen.getByRole("button", { name: /cloud/i });
    for (const el of [newCard, openCard, cloud]) expect(el.classList.contains("vf")).toBe(true);
    expect(newCard.hasAttribute("data-selected")).toBe(false);
    await fireEvent.click(newCard);
    expect(newCard.hasAttribute("data-selected")).toBe(true);
  });

  it("the Name input has no placeholder", async () => {
    await renderPage();
    await fireEvent.keyDown(window, { key: "n", ctrlKey: true });
    const name = await screen.findByLabelText("Name");
    expect(name.hasAttribute("placeholder")).toBe(false);
    expect(document.querySelector("[placeholder]")).toBeNull();
  });

  it("a missing recent shows a warning tally dot and 'Missing'", async () => {
    h.api.listRecents.mockResolvedValue([recents[0], { ...recents[1], missing: true }]);
    await renderPage();
    const list = screen.getByRole("list", { name: "Recent projects" });
    const missing = within(list).getAllByRole("listitem")[1];
    const status = within(missing).getByText("Missing");
    expect(status.querySelector(".tally.warning")).toBeTruthy();
    expect(within(missing).queryByRole("button", { name: /gone show/i })).toBeNull();
    expect(within(missing).getByRole("button", { name: "Remove" })).toBeTruthy();
    expect(within(list).getAllByRole("listitem")[0].querySelector(".tally")).toBeNull();
  });
});
