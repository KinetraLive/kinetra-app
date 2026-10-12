// Control's live commands, registered while the page is mounted (key scope "control"): thin
// wrappers around the same ControlCtl actions the buttons use, no logic of their own.
//   live.toggle:<screenId>/<layerId>   Take / Clear a layer (Cue in Studio mode); its key is the
//                                      layer's hotkey (control.hotkeys "<screenId>/<layerId>").
//                                      Hidden in the palette: the provider below lists the explicit
//                                      action instead (the row shows this key).
//   cue.go:<flowId>                    GO a cue (run again = restart or ignored, per flow); its key
//                                      is the cue's hotkey ("flow/<flowId>")
//   live.panic                         PANIC, Shift+Escape (works while typing; the palette asks
//                                      for a second Enter)
//   live.clearAll                      Clear all screens (animated out; a second Enter)
// Live commands ignore key repeats and, except PANIC, never fire while typing. Nothing fires while
// "Set hotkey" waits for a key. Re-registered only when the layers, cues or their names change
// (not on every live update: titles read the live state when shown).
//
// The provider "control.live" (asked only while the palette is open, in Control): for each visible
// screen, the selected one first (rank +11 000, the others +10 000, so live actions come first),
// its layers front first like the layer panel:
//   live.take:<s>/<l>      Take {layer}        off air            ctl.take
//   live.clear:<s>/<l>     Clear {layer}       on air             ctl.clear
//   live.update:<s>/<l>    Update {layer}      on air, changes staged for the next take: ctl.take
//   live.cue:<s>/<l>       Cue {layer}         Studio mode (instead of the three above): ctl.toggle
//   live.clearScreen:<s>   Clear {screen}      something on air   ctl.clearScreen
//   media.play:<s>/<l>     Play {layer}        a media layer not playing   media_control (Program)
//   media.pause:<s>/<l>    Pause {layer}       a media layer playing
//   media.stop:<s>/<l>     Stop {layer}        a media layer not stopped
// Layers of the other screens say which screen: "Take {layer} ({screen})". Single-layer actions
// and Clear {screen} run at once; only Clear all and PANIC ask for a second Enter.

import { untrack } from "svelte";
import { provideContext, registerCommands, registerProvider, type Command, type CommandContext, type CommandProvider } from "$lib/commands/registry.svelte";
import type { FlowInfo, LiveLayerView } from "$lib/types";
import type { ControlCtl } from "./control.svelte";

interface LayerRef {
  screenId: string;
  screen: string;
  layerId: string;
  name: string;
}

export function controlCommands(ctl: ControlCtl, layers: readonly LayerRef[], cues: readonly { id: string; label: string }[]): Command[] {
  const idle = () => ctl.capturing === null;
  const out: Command[] = [
    {
      id: "live.panic",
      title: "Panic: cut everything off air",
      section: "live",
      keyScope: "control",
      defaultKeys: ["Shift+Escape"],
      live: true,
      whileTyping: true,
      confirm: true,
      run: () => ctl.panic(),
    },
    {
      id: "live.clearAll",
      title: "Clear all screens",
      section: "live",
      keyScope: "control",
      live: true,
      confirm: true,
      when: idle,
      rank: (ctx) => (ctx.route === "control" ? LIVE_RANK : 0),
      run: () => ctl.clearEvery(),
    },
  ];
  for (const l of layers) {
    out.push({
      id: `live.toggle:${l.screenId}/${l.layerId}`,
      get title() {
        return ctl.studio.on ? "Cue {layer}" : "Take or clear {layer}";
      },
      params: () => ({ layer: l.name, screen: l.screen }),
      keywords: [l.screen],
      section: "live",
      keyScope: "control",
      live: true,
      hidden: true,
      when: idle,
      run: () => ctl.toggle(l.screenId, l.layerId),
    });
  }
  for (const c of cues) {
    out.push({
      id: `cue.go:${c.id}`,
      title: "GO {cue}",
      params: () => ({ cue: c.label }),
      section: "live",
      keyScope: "control",
      live: true,
      when: idle,
      rank: (ctx) => (ctx.route === "control" ? LIVE_RANK : 0),
      run: () => ctl.flows.run(c.id),
    });
  }
  return out;
}

const LIVE_RANK = 10_000;
/** The selected screen's items rank above the other screens'. */
const FOCUS_RANK = 1_000;

/** The palette's live items for Control's screens (see the header). */
export function controlProvider(ctl: ControlCtl): CommandProvider {
  return {
    id: "control.live",
    items(ctx: CommandContext): Command[] {
      if (ctx.route !== "control" || ctl.capturing !== null) return [];
      const studio = ctl.studio.on;
      const views = [...ctl.visible].sort((a, b) => Number(b.screen.id === ctx.screenId) - Number(a.screen.id === ctx.screenId));
      const out: Command[] = [];
      for (const v of views) {
        const s = v.screen.id;
        const focused = s === ctx.screenId;
        const rank = () => LIVE_RANK + (focused ? FOCUS_RANK : 0);
        const base = { section: "live", live: true, rank, keywords: [v.screen.name] } as const;
        const item = (id: string, title: string, l: LiveLayerView, run: () => Promise<void>): Command => ({
          ...base,
          id: `${id}:${s}/${l.id}`,
          title: focused ? title : `${title} ({screen})`,
          params: () => ({ layer: l.name, screen: v.screen.name }),
          run,
        });
        // Front first, like Control's layer panel.
        for (const l of [...v.layers].reverse()) {
          // Media transport acts on Program at once, in Studio mode too.
          const pb = l.media?.playback.state;
          if (pb) {
            const media = (action: "play" | "pause" | "stop") => async () => {
              const { mediaControl } = await import("$lib/api/media");
              await mediaControl(s, l.id, action);
            };
            if (pb !== "playing") out.push(item("media.play", "Play {layer}", l, media("play")));
            else out.push(item("media.pause", "Pause {layer}", l, media("pause")));
            if (pb !== "stopped") out.push(item("media.stop", "Stop {layer}", l, media("stop")));
          }
          if (studio) {
            out.push(item("live.cue", "Cue {layer}", l, () => ctl.toggle(s, l.id)));
            continue;
          }
          if (!l.onAir) out.push(item("live.take", "Take {layer}", l, () => ctl.take(s, l.id)));
          else {
            if (l.pending) out.push(item("live.update", "Update {layer}", l, () => ctl.take(s, l.id)));
            out.push(item("live.clear", "Clear {layer}", l, () => ctl.clear(s, l.id)));
          }
        }
        if (v.layers.some((l) => l.onAir)) {
          out.push({
            ...base,
            id: `live.clearScreen:${s}`,
            title: "Clear {screen}",
            params: () => ({ screen: v.screen.name }),
            run: () => ctl.clearScreen(s),
          });
        }
      }
      return out;
    },
  };
}

/**
 * Registers Control's commands and its palette provider and keeps them current; returns the
 * function that removes them. `cueLabel`: a cue's button text (passed in, so this lazy chunk
 * imports only shell modules).
 */
export function watchControlCommands(ctl: ControlCtl, cueLabel: (f: FlowInfo) => string): () => void {
  let off = () => {};
  let last = "";
  const offProvider = registerProvider(controlProvider(ctl));
  const stop = $effect.root(() => {
    // The command context's layerIds: the selected screen's expanded rows (the menu's Show › Take /
    // Clear act on exactly one). Here, not in the page: this chunk is lazy.
    $effect(() => {
      const s = ctl.selectedId;
      const layerIds = s ? [...ctl.expanded].filter((k) => k.startsWith(s + "/")).map((k) => k.slice(s.length + 1)) : [];
      return untrack(() => provideContext({ layerIds }));
    });
    $effect(() => {
      const layers: LayerRef[] = [];
      for (const v of ctl.screens) {
        for (const l of v.layers) layers.push({ screenId: v.screen.id, screen: v.screen.name, layerId: l.id, name: l.name });
      }
      const cues = ctl.flows.cues.map((f) => ({ id: f.id, label: cueLabel(f) }));
      const key = JSON.stringify([layers, cues]);
      if (key === last) return;
      last = key;
      untrack(() => {
        off();
        off = registerCommands(controlCommands(ctl, layers, cues));
      });
    });
  });
  return () => {
    stop();
    off();
    offProvider();
  };
}
