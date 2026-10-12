// The lazy studio chunk, loaded once Studio mode is on: takes and cues (./ops.ts), the single
// view's studio stage, the layer rows' studio parts, the toolbar's settings menu, the exit question
// and the palette commands "Take preview", "Take preview (<screen>)" and "Transition: …".

import { untrack } from "svelte";
import { registerCommands, type Command } from "$lib/commands/registry.svelte";
import { t } from "$lib/i18n";
import type { ControlCtl } from "../control.svelte";
import { TAKE_KINDS } from "./transitions";

export { default as StudioStage } from "./StudioStage.svelte";
export { default as StudioRow } from "./StudioRow.svelte";
export { default as StudioExit } from "./StudioExit.svelte";
export { default as StudioSettingsMenu } from "./StudioSettingsMenu.svelte";
export { cue, take, clearPreview, turnOff } from "./ops";
export { previews } from "$lib/stores/preview.svelte";

/** Studio commands: in Control only. */
const BASE = { keyScope: "control" } as const;

/** The studio commands for the visible screens; returns the function that removes them. */
export function registerStudioCommands(ctl: ControlCtl): () => void {
  let off = () => {};
  let last = "";
  const stop = $effect.root(() => {
    $effect(() => {
      const screens = ctl.visible.map((v) => ({ id: v.screen.id, name: v.screen.name }));
      const key = JSON.stringify(screens);
      if (key === last) return;
      last = key;
      untrack(() => {
        off();
        off = registerCommands(commands(ctl, screens));
      });
    });
  });
  return () => {
    stop();
    off();
  };
}

function commands(ctl: ControlCtl, screens: { id: string; name: string }[]): Command[] {
  const s = ctl.studio;
  const out: Command[] = [
    {
      id: "studio.take",
      title: "Take preview",
      ...BASE,
      section: "live",
      live: true,
      run: () => {
        if (ctl.selectedId) void s.take(ctl.selectedId);
      },
      when: () => s.on,
      disabled: () => (ctl.selectedId !== null ? null : t("Select a screen first")),
    },
  ];
  for (const sc of screens) {
    out.push({
      id: `studio.take:${sc.id}`,
      title: "Take preview ({screen})",
      params: () => ({ screen: sc.name }),
      ...BASE,
      section: "live",
      live: true,
      run: () => void s.take(sc.id),
      when: () => s.on,
    });
  }
  for (const k of TAKE_KINDS) {
    out.push({
      id: `studio.transition:${k.value}`,
      title: "Transition: {kind}",
      params: () => ({ kind: t(k.label) }),
      ...BASE,
      section: "settings",
      run: () => s.setTransition({ ...s.transition, kind: k.value }),
      when: () => s.on,
    });
  }
  return out;
}
