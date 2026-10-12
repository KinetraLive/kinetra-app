<!--
  MidiTriggerFields: the settings of a "MIDI in" flow trigger (the node inspector and the linear
  editor): device (Any device or one by name), channel (Any or 1–16), Note or CC, its number
  (0–127, mono) and Learn (the next MIDI message fills all four; Esc cancels).

  Props:
    config: { device, channel, kind, number }
    onchange: (config) => void
    disabled?: boolean
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { listMidiDevices } from "$lib/api/midi";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import type { MidiSource } from "$lib/types";
  import { Learn, LEARN_HINT } from "./learn.svelte";
  import { sourceOf } from "./model";

  let { config, onchange, disabled = false }: { config: MidiSource; onchange: (c: MidiSource) => void; disabled?: boolean } = $props();

  let inputs = $state.raw<string[]>([]);
  const learn = new Learn();
  onMount(() => {
    listMidiDevices().then(
      (d) => (inputs = d.filter((x) => x.input).map((x) => x.name)),
      () => {},
    );
    return () => learn.stop();
  });

  const deviceOptions = $derived([
    { value: "", label: "Any device" },
    ...[...new Set([...inputs, ...(config.device ? [config.device] : [])])].map((n) => ({ value: n, label: n })),
  ]);
  const CHANNELS = [{ value: "", label: "Any channel" }, ...Array.from({ length: 16 }, (_, i) => ({ value: String(i + 1), label: `Ch ${i + 1}` }))];
  const KINDS: { value: MidiSource["kind"]; label: string }[] = [
    { value: "note", label: "Note" },
    { value: "cc", label: "CC (knob, fader)" },
  ];
</script>

<span class="midi">
  <Select size="sm" aria-label="MIDI device" {disabled} value={config.device ?? ""} options={deviceOptions} onchange={(v: string) => onchange({ ...config, device: v || null })} />
  <Select size="sm" aria-label="MIDI channel" {disabled} value={config.channel === null ? "" : String(config.channel)} options={CHANNELS} onchange={(v: string) => onchange({ ...config, channel: v ? Number(v) : null })} />
  <Select size="sm" aria-label="Note or CC" {disabled} value={config.kind} options={KINDS} onchange={(v) => onchange({ ...config, kind: v })} />
  <NumberInput class="mono" aria-label={config.kind === "cc" ? "CC number" : "Note number"} {disabled} value={config.number} min={0} max={127} digits={3} onchange={(n) => onchange({ ...config, number: n })} />
  {#if learn.active}
    <span class="hint" role="status">{LEARN_HINT}</span>
    <Button size="sm" variant="ghost" onclick={() => learn.stop()}>Cancel</Button>
  {:else}
    <Button size="sm" {disabled} onclick={() => learn.start((l) => onchange(sourceOf(l)))}>Learn</Button>
  {/if}
</span>

<style>
  .midi {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .hint {
    color: var(--accent-text);
    font-size: var(--fs-sm);
  }
</style>
