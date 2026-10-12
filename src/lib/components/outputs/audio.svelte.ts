// Audio output devices for Preferences › Outputs, by label (what OutputConfig.audio.device stores:
// device ids differ per web origin, labels don't). Listed with `enumerateDevices()`; WebView2 shows
// labels only once the page's origin has the microphone permission state, which the app grants at
// startup without capturing anything. Refreshed on `devicechange` while a list is watched.

import type { OutputConfig } from "$lib/types";

/** Labels of the audio outputs ("default" / "communications" aliases dropped), sorted, unique. */
export function outputLabels(devices: readonly Pick<MediaDeviceInfo, "kind" | "deviceId" | "label">[]): string[] {
  const out = new Set<string>();
  for (const d of devices) {
    if (d.kind !== "audiooutput" || !d.label) continue;
    if (d.deviceId === "default" || d.deviceId === "communications") continue;
    out.add(d.label);
  }
  return [...out].sort((a, b) => a.localeCompare(b));
}

/** An output's audio as the UI shows it (absent = on, the system default; key outputs never). */
export function audioOf(o: Pick<OutputConfig, "mode" | "audio">): { enabled: boolean; device: string | null } {
  if (o.mode === "key") return { enabled: false, device: null };
  return o.audio ?? { enabled: true, device: null };
}

/** Select options: System default, the devices, and the saved one when it isn't connected. */
export function deviceOptions(labels: readonly string[], saved: string | null): { value: string; label: string }[] {
  const opts = [{ value: "", label: "System default" }, ...labels.map((l) => ({ value: l, label: l }))];
  if (saved && !labels.includes(saved)) opts.push({ value: saved, label: `${saved} (not connected)` });
  return opts;
}

class AudioDevices {
  labels = $state.raw<string[]>([]);
  /** False when the list can't be read here. */
  available = $state(true);
  #watchers = 0;

  async refresh() {
    const md = typeof navigator !== "undefined" ? navigator.mediaDevices : undefined;
    if (!md?.enumerateDevices) {
      this.available = false;
      return;
    }
    try {
      this.labels = outputLabels(await md.enumerateDevices());
      this.available = true;
    } catch {
      this.available = false;
    }
  }

  #onChange = () => void this.refresh();

  /** Lists the devices while watched (an effect's cleanup returns it). */
  watch(): () => void {
    const md = typeof navigator !== "undefined" ? navigator.mediaDevices : undefined;
    if (this.#watchers++ === 0) {
      void this.refresh();
      md?.addEventListener?.("devicechange", this.#onChange);
    }
    return () => {
      if (--this.#watchers === 0) md?.removeEventListener?.("devicechange", this.#onChange);
    };
  }
}

export const audioDevices = new AudioDevices();
