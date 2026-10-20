// Preferences › Cloud backup › Cloud projects: the account's projects, Rename, Move to trash
// (confirmed), the trash with Restore.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CloudProjectSummary } from "$lib/types";

const api = vi.hoisted(() => ({
  cloudProjectsList: vi.fn(),
  cloudProjectRename: vi.fn(async () => null),
  cloudProjectTrash: vi.fn(async () => null),
  cloudProjectUntrash: vi.fn(async () => null),
}));
vi.mock("$lib/api/cloud-history", () => api);
vi.mock("$lib/api", () => ({ toAppError: (e: unknown) => ({ message: String(e) }) }));

import CloudProjects from "./cloud/CloudProjects.svelte";

const p = (patch: Partial<CloudProjectSummary>): CloudProjectSummary => ({
  id: "p1",
  name: "Gala",
  versions: 41,
  kept: 3,
  conflict: false,
  lastAt: "2026-10-15T12:00:00Z",
  lastMachine: "STUDIO-PC",
  trashedAt: null,
  purgeAt: null,
  open: true,
  here: true,
  ...patch,
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const rowOf = (name: string) => screen.getByText(name).closest("li") as HTMLElement;

describe("cloud projects", () => {
  it("lists them and renames one", async () => {
    api.cloudProjectsList.mockResolvedValue([p({}), p({ id: "p2", name: "Election", open: false })]);
    render(CloudProjects);
    expect(await screen.findByText("Gala")).toBeTruthy();
    expect(within(rowOf("Gala")).getByText("This project")).toBeTruthy();
    expect(within(rowOf("Gala")).getByText("Last backup Oct 15, 2026 from STUDIO-PC · 41 versions")).toBeTruthy();
    await fireEvent.click(within(rowOf("Election")).getByRole("button", { name: "Rename" }));
    const input = screen.getByRole("textbox", { name: "Name" });
    await fireEvent.input(input, { target: { value: "Election night" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.cloudProjectRename).toHaveBeenCalledWith("p2", "Election night"));
  });

  it("moves one to the trash after a confirm, and restores from the trash", async () => {
    api.cloudProjectsList.mockResolvedValue([p({})]);
    render(CloudProjects);
    await screen.findByText("Gala");
    await fireEvent.click(within(rowOf("Gala")).getByRole("button", { name: "Move to trash" }));
    expect(screen.getByText("Move to the trash? It's deleted for good after 7 days.")).toBeTruthy();
    const confirm = screen.getAllByRole("button", { name: "Move to trash" }).find((b) => b.classList.contains("danger") && b.closest("[role=group]"));
    await fireEvent.click(confirm!);
    await waitFor(() => expect(api.cloudProjectTrash).toHaveBeenCalledWith("p1"));
    // The trash.
    api.cloudProjectsList.mockResolvedValue([p({ trashedAt: "2026-10-20T10:00:00Z", purgeAt: "2026-10-27T10:00:00Z" })]);
    await fireEvent.click(screen.getByRole("switch", { name: "Show the trash" }));
    await waitFor(() => expect(api.cloudProjectsList).toHaveBeenLastCalledWith(true));
    expect(await screen.findByText("In the trash · deleted on Oct 27, 2026")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Restore" }));
    await waitFor(() => expect(api.cloudProjectUntrash).toHaveBeenCalledWith("p1"));
  });
});
