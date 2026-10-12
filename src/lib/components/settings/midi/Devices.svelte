<!--
  Preferences > MIDI & OSC > Devices (part of the section's lazy chunk). One row per MIDI input
  this machine has or remembers: name, status (TallyDot + "Connected", "Not connected", or "In use
  by another app" when it can't be opened), an on/off switch (prefs.json `midi.inputs`; a device
  not listed is on), LED feedback (Off, or an output port: the device itself first) with its
  colour profile, and "Test lights". The list follows `midi-devices` (hot-plug). Device settings
  are per machine (the prefs store).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { toAppError } from "$lib/api";
  import { listMidiDevices, onMidiDevices, testMidiFeedback, watchMidiDevices } from "$lib/api/midi";
  import Button from "$lib/components/Button.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { MidiDevice, MidiFeedbackProfile, MidiPrefs } from "$lib/types";
  import { deviceRows, outputOptions, PROFILE_OPTIONS, withFeedback, withInput } from "./devices";

  let devices = $state.raw<readonly MidiDevice[]>([]);
  let loaded = $state(false);
  let testing = $state<string | null>(null);

  onMount(() => {
    const off = onMidiDevices((d) => {
      devices = d;
      loaded = true;
    });
    listMidiDevices().then(
      (d) => {
        devices = d;
        loaded = true;
      },
      () => (loaded = true),
    );
    // Ports open and hot-plug is watched only while this list is shown.
    void watchMidiDevices(true).catch(() => {});
    return () => {
      off();
      void watchMidiDevices(false).catch(() => {});
    };
  });

  const midi = $derived<MidiPrefs>(prefs.value?.midi ?? { inputs: {}, feedback: {} });
  const rows = $derived(deviceRows(devices, midi));

  async function write(next: MidiPrefs) {
    const p = prefs.value;
    if (!p) return;
    try {
      await prefs.update({ ...p, midi: next });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't save the device setting: ${toAppError(e).message}` });
    }
  }

  async function test(name: string) {
    testing = name;
    try {
      await testMidiFeedback(name);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't light ${name}: ${toAppError(e).message}` });
    } finally {
      testing = null;
    }
  }
</script>

<div class="devices">
  {#if loaded && rows.length === 0}
    <p class="empty">Connect a MIDI controller. It shows up here at once.</p>
  {:else}
    <ul class="rows" aria-label="MIDI devices">
      {#each rows as r (r.name)}
        {@const fb = midi.feedback[r.name] ?? null}
        <li class="row" data-device={r.name}>
          <span class="name">
            <span class="title">{r.name}</span>
            <span class="status">
              <TallyDot state={r.status === "connected" ? "preview" : r.status === "busy" ? "warning" : "idle"} />
              {r.status === "connected" ? "Connected" : r.status === "busy" ? "In use by another app" : "Not connected"}
            </span>
          </span>
          <Switch label="Use {r.name}" hideLabel checked={r.enabled} onchange={(on) => void write(withInput(midi, r.name, on))} />
          <span class="fb">
            <Select
              size="sm"
              aria-label="LED feedback of {r.name}"
              value={fb ? (fb.output ?? "") : "off"}
              options={outputOptions(devices, r.name, fb ? fb.output : undefined)}
              onchange={(v: string) => void write(withFeedback(midi, r.name, v === "off" ? null : { output: v === "" ? null : v, profile: fb?.profile ?? "generic" }))}
            />
            {#if fb}
              <Select
                size="sm"
                aria-label="Light colours of {r.name}"
                value={fb.profile}
                options={PROFILE_OPTIONS}
                onchange={(p: MidiFeedbackProfile) => void write(withFeedback(midi, r.name, { ...fb, profile: p }))}
              />
            {/if}
          </span>
          <Button size="sm" disabled={!fb || r.status !== "connected"} loading={testing === r.name} onclick={() => void test(r.name)}>
            Test lights
          </Button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .rows {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: grid;
    /* Same columns in every row (each row is its own grid). */
    grid-template-columns: minmax(10rem, 1fr) auto 20rem auto;
    align-items: center;
    gap: var(--s-1) var(--s-3);
    min-height: var(--row-h);
    padding: var(--s-1) 0;
    border-top: 1px solid var(--line);
  }
  .row:last-child {
    border-bottom: 1px solid var(--line);
  }
  .name {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.3;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .fb {
    display: inline-flex;
    flex-wrap: wrap;
    gap: var(--s-1);
    min-width: 0;
  }
  .empty {
    margin: 0;
    padding: var(--s-2) 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  @container (max-width: 560px) {
    .row {
      grid-template-columns: minmax(0, 1fr) auto;
    }
    .fb {
      grid-column: 1 / -1;
    }
  }
</style>
