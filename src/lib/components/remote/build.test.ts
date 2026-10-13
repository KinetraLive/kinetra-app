// Builds without remote control (`$remote` → build-off.ts, the default outside `pnpm dev`): no
// switch, no chip, no Preferences block, no palette command or Preferences entry.
import { describe, expect, it } from "vitest";
import * as remoteBuild from "$remote";
import * as off from "./build-off";
import { PREFERENCE_ENTRIES } from "$lib/components/settings/sections";
import { shellCommands } from "$lib/components/shell/commands";

describe("a build without remote control", () => {
  it("resolves $remote to the empty module", () => {
    expect(remoteBuild.REMOTE_BUILD).toBe(false);
    expect({ ...off }).toEqual({ REMOTE_BUILD: false, loadChip: null, loadSection: null, setRemote: null });
    expect(remoteBuild.loadChip).toBeNull();
  });

  it("lists no Remote control entry and no switch command", () => {
    expect(PREFERENCE_ENTRIES.some((e) => e.id === "remote")).toBe(false);
    const noop = () => {};
    const cmds = shellCommands({ openPalette: noop, go: noop, toggleAutomation: noop, automationOff: () => false, remoteOn: () => false });
    expect(cmds.some((c) => c.id === "shell.remote")).toBe(false);
    const withRemote = shellCommands({ openPalette: noop, go: noop, toggleAutomation: noop, automationOff: () => false, remoteOn: () => false, toggleRemote: noop });
    expect(withRemote.some((c) => c.id === "shell.remote")).toBe(true);
  });
});
