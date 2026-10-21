import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import LicensesDialog from "./LicensesDialog.svelte";

afterEach(cleanup);

async function open() {
  const onclose = vi.fn();
  render(LicensesDialog, { onclose });
  const dialog = await screen.findByRole("dialog", { name: "Third-party licenses", hidden: true });
  await within(dialog).findByRole("heading", { name: /App \(npm\)/ }, { timeout: 5_000 });
  return { dialog, onclose };
}

const rows = (dialog: HTMLElement) => [...dialog.querySelectorAll<HTMLButtonElement>("button.row")];

describe("LicensesDialog", () => {
  it("lists the groups with counts, names, mono versions and license badges", async () => {
    const { dialog } = await open();
    const labels = [...dialog.querySelectorAll("h3")].map((h) => h.textContent!.replace(/\s+/g, " ").trim());
    expect(labels).toHaveLength(4);
    expect(labels[0]).toMatch(/^App \(npm\) \d+$/);
    expect(labels[1]).toMatch(/^Core \(Rust\) \d+$/);
    expect(labels[2]).toMatch(/^Fonts \d+$/);
    expect(labels[3]).toBe("NDI® 2");
    const xy = rows(dialog).find((r) => r.querySelector(".name")?.textContent === "@xyflow/svelte")!;
    expect(xy).toBeTruthy();
    expect(xy.querySelector(".version")!.classList.contains("mono")).toBe(true);
    expect(xy.textContent).toContain("MIT");
    expect(xy.getAttribute("aria-expanded")).toBe("false");
  });

  it("searches, expands a row to its license text and collapses it again", async () => {
    const { dialog } = await open();
    const search = within(dialog).getByRole("searchbox", { name: "Search licenses" });
    expect(document.activeElement).toBe(search);
    await fireEvent.input(search, { target: { value: "xyflow" } });
    expect(rows(dialog).map((r) => r.querySelector(".name")!.textContent)).toEqual(["@xyflow/svelte", "@xyflow/system"]);
    expect(dialog.querySelectorAll("h3")).toHaveLength(1);
    expect(dialog.textContent).toContain("2 of");

    const row = rows(dialog)[0];
    await fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    const panel = document.getElementById(row.getAttribute("aria-controls")!)!;
    expect(panel.textContent).toContain("https://github.com/xyflow/xyflow");
    expect(panel.querySelector("pre")!.textContent).toMatch(/^MIT License[\s\S]*webkid GmbH/);
    await fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("false");
    expect(document.getElementById(row.getAttribute("aria-controls")!)).toBeNull();
  });

  it("offers to clear a search that matches nothing", async () => {
    const { dialog } = await open();
    const search = within(dialog).getByRole("searchbox", { name: "Search licenses" });
    await fireEvent.input(search, { target: { value: "no-such-package-zz" } });
    expect(rows(dialog)).toHaveLength(0);
    expect(dialog.textContent).toContain("No package matches “no-such-package-zz”.");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Clear search" }));
    expect(rows(dialog).length).toBeGreaterThan(300);
  });

  it("closes from the close button and from Esc", async () => {
    const { dialog, onclose } = await open();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    expect(onclose).toHaveBeenCalledTimes(1);
    await fireEvent(dialog, new Event("cancel", { cancelable: true }));
    expect(onclose).toHaveBeenCalledTimes(2);
  });
});
