import { render, screen } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import BrandLogo from "./BrandLogo.svelte";
import CanaryBadge from "./CanaryBadge.svelte";

describe("BrandLogo", () => {
  it("shows the logo for each theme and, in a stable build, no channel badge", () => {
    const { container } = render(BrandLogo, { height: 32 });
    const imgs = [...container.querySelectorAll("img")];
    expect(imgs.map((i) => i.className.split(" ")[0])).toEqual(["for-dark", "for-light"]);
    expect(imgs.every((i) => i.alt === "" && i.getAttribute("height") === "32" && i.getAttribute("width") === "128")).toBe(true);
    // One accessible name for the pair, whichever theme image shows.
    expect(screen.getAllByRole("img", { name: "Kinetra" })).toHaveLength(1);
    expect(imgs[0].getAttribute("src")).toMatch(/kinetra-logo-for-dark-bg/);
    expect(imgs[1].getAttribute("src")).toMatch(/kinetra-logo-for-light-bg/);
    expect(container.textContent?.toLowerCase()).not.toContain("canary");
  });
});

describe("CanaryBadge (canary builds only)", () => {
  it("says Canary with the experimental-build warning as its tooltip", () => {
    render(CanaryBadge);
    const badge = screen.getByText("Canary");
    expect(badge.classList.contains("canary-badge")).toBe(true);
    expect(document.body.textContent).toContain("Experimental build. Don't use for live shows.");
  });
});
