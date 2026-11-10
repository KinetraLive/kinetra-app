// The Generate template dialog's brand colors (`pnpm shots:ui`): each color is a picker and a hex
// field kept in sync; typed hex (#E50914, E50914, 3 or 6 digits) updates the swatch live, an
// invalid entry is marked and holds Generate. Screenshots into review/ai-colors/.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { loadTemplates, open } from "./harness";

const OUT = join(process.cwd(), "review", "ai-colors");
mkdirSync(OUT, { recursive: true });
const data = loadTemplates();

test.use({ viewport: { width: 1280, height: 800 } });

test("brand colors take typed hex, in sync with the picker", async ({ page }) => {
  await open(page, "/templates", {
    theme: "dark",
    screens: [],
    lanAccess: false,
    selectedScreenId: null,
    testOnAir: false,
    templates: data.templates,
    presets: data.presets,
    brand: { accent: "#C8102E", secondary: "#ffffff" },
  });
  await page.getByRole("button", { name: "Generate template" }).first().click();
  const dialog = page.getByRole("dialog", { name: "Generate template" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/left this month/)).toBeVisible();
  await dialog.getByLabel("Describe the graphic").fill("Lower third for a football match, club colors, bold names");
  const shot = (name: string) => dialog.screenshot({ path: join(OUT, `${name}.png`) });
  await shot("brand-colors");

  const hex = dialog.getByLabel("Brand color 2 hex");
  const picker = dialog.getByLabel("Brand color 2", { exact: true });
  const generate = dialog.getByRole("button", { name: "Generate" });

  await hex.fill("#E509");
  await expect(hex).toHaveAttribute("aria-invalid", "true");
  await expect(dialog.getByText("Use a hex color, like #E50914 or E50914.")).toBeVisible();
  await expect(generate).toBeDisabled();
  await shot("brand-colors-invalid");

  await hex.fill("E50914");
  await expect(picker).toHaveValue("#e50914");
  await expect(generate).toBeEnabled();
  await hex.blur();
  await expect(hex).toHaveValue("#E50914");

  await dialog.getByRole("button", { name: "Add color" }).click();
  await dialog.getByLabel("Brand color 3 hex").fill("0a3");
  await expect(dialog.getByLabel("Brand color 3", { exact: true })).toHaveValue("#00aa33");
  await dialog.getByLabel("Brand color 3 hex").blur();
  await shot("brand-colors-valid");
});
