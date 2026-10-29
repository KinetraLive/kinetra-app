<!--
  Preferences › Outputs: one output. The head: an on/off switch, the name (edited in place, saved
  on change), its status (a red TallyDot + "Showing" while on air; amber text with an icon for a
  problem; muted otherwise; a key pair adds "Frame-locked" or "Not frame-locked: displays differ")
  and Delete. Below: Display (connected displays and the remembered one
  when it is disconnected), Screen (a key output shows its fill's), Mode Fill / Key, Background
  (fill: Black / Green / Color) or Key for (key: a fill output), Sound (fill outputs: on/off and the
  audio device by label, "System default" first; key outputs are silent; macOS: on/off only, the
  system's output, WebKit can't choose a device), Always on top, and the test patterns (pressed
  while showing; Back to screen). A macOS display in a scaled mode gets a note (not pixel exact).

  Props:
    output: OutputConfig
    outputs: OutputConfig[]         all of them (key pairs, edits)
    status?: OutputStatus
    displays: DisplayInfo[]
    screens: { id, name }[] | null  null while no project is open
    onsave: (next: OutputConfig[]) => void
    onpattern: (pattern: TestPattern | null) => void
-->
<script lang="ts">
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import Button from "$lib/components/Button.svelte";
  import Segmented from "$lib/components/control/Segmented.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import {
    BACKGROUNDS,
    PATTERNS,
    displayIdOf,
    displayOptions,
    displayRef,
    fillOptions,
    frameLockText,
    patchOutput,
    removeOutput,
    statusLine,
    withBackground,
    withMode,
    type BackgroundKind,
  } from "$lib/components/outputs/model";
  import { audioDevices, audioOf, deviceOptions } from "$lib/components/outputs/audio.svelte";
  import { isMac } from "$lib/platform";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { DisplayInfo, OutputConfig, OutputStatus, TestPattern } from "$lib/types";

  type Props = {
    output: OutputConfig;
    outputs: readonly OutputConfig[];
    status?: OutputStatus;
    displays: readonly DisplayInfo[];
    screens: readonly { id: string; name: string }[] | null;
    onsave: (next: OutputConfig[]) => void;
    onpattern: (pattern: TestPattern | null) => void;
  };
  let { output: o, outputs, status, displays, screens, onsave, onpattern }: Props = $props();

  const uid = $props.id();
  const MODES = [
    { value: "fill", label: "Fill" },
    { value: "key", label: "Key" },
  ] as const;

  const line = $derived(statusLine(o, status));
  const lock = $derived(o.mode === "key" || outputs.some((k) => k.keyOf === o.id) ? frameLockText(status) : null);
  const fill = $derived(o.mode === "key" ? outputs.find((f) => f.id === o.keyOf) : undefined);
  let name = $derived(o.name);

  const patch = (p: Partial<OutputConfig>) => onsave(patchOutput(outputs, o.id, p));

  const mac = isMac();
  const audio = $derived(audioOf(o));
  $effect(() => (o.mode === "fill" && !mac ? audioDevices.watch() : undefined));
  const scaled = $derived(displays.find((d) => d.id === displayIdOf(o, status))?.scaled === true);

  function rename() {
    const next = name.trim().slice(0, 40);
    if (!next) {
      name = o.name;
      return;
    }
    if (outputs.some((c) => c.id !== o.id && c.name.toLowerCase() === next.toLowerCase())) {
      toasts.push({ kind: "error", message: `There is already an output named “${next}”.` });
      name = o.name;
      return;
    }
    if (next !== o.name) patch({ name: next });
  }

  function setDisplay(id: string) {
    const d = displays.find((x) => x.id === id);
    if (d) patch({ display: displayRef(d) });
  }

  function setMode(mode: OutputConfig["mode"]) {
    if (mode === o.mode) return;
    const next = withMode(outputs, o.id, mode);
    if (next) onsave(next);
    else toasts.push({ kind: "error", message: "Add a fill output first." });
  }

  const screenOptions = $derived.by(() => {
    if (!screens) return [{ value: "", label: "Open a project to choose a screen" }];
    const opts = [{ value: "", label: "No screen" }, ...screens.map((s) => ({ value: s.id, label: s.name }))];
    if (o.screenId && !screens.some((s) => s.id === o.screenId)) opts.push({ value: o.screenId, label: "Screen not found" });
    return opts;
  });
</script>

<li class="output" data-output={o.id} aria-labelledby="{uid}-name">
  <div class="head">
    <Switch label="Show {o.name}" hideLabel checked={o.enabled} onchange={(on) => patch({ enabled: on })} />
    <span class="field vf vf-within name">
      <input
        id="{uid}-name"
        type="text"
        aria-label="Output name"
        maxlength="40"
        spellcheck="false"
        autocomplete="off"
        bind:value={name}
        onchange={rename}
        onkeydown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
    </span>
    <span class="status {line.tone}" role="status">
      {#if line.tone === "program"}
        <TallyDot state="program" />
      {:else if line.tone === "warning"}
        <TriangleAlert size={14} aria-hidden="true" />
      {/if}
      {line.text}
      {#if status?.message && line.tone !== "warning"}<span class="note">· {status.message}</span>{/if}
      {#if lock}<span class="note" class:warn={status?.frameLocked === false}>· {lock}</span>{/if}
    </span>
    <Button size="sm" variant="danger" onclick={() => onsave(removeOutput(outputs, o.id))}>Delete</Button>
  </div>

  <div class="fields">
    <label class="lbl" for="{uid}-display">Display</label>
    <span class="ctl">
      <Select id="{uid}-display" size="sm" value={displayIdOf(o, status)} options={displayOptions(o, status, displays)} onchange={setDisplay} />
      {#if scaled}<span class="note">Scaled display mode: not pixel exact.</span>{/if}
    </span>

    <label class="lbl" for="{uid}-screen">Screen</label>
    {#if o.mode === "key"}
      <span class="inherited">{fill ? `Same as ${fill.name}` : "Same as its fill"}</span>
    {:else}
      <Select
        id="{uid}-screen"
        size="sm"
        disabled={!screens}
        value={screens ? (o.screenId ?? "") : ""}
        options={screenOptions}
        onchange={(v: string) => patch({ screenId: v || null })}
      />
    {/if}

    <span class="lbl">Mode</span>
    <span class="ctl"><Segmented label="Mode of {o.name}" options={MODES} value={o.mode} onchange={setMode} /></span>

    {#if o.mode === "key"}
      <label class="lbl" for="{uid}-keyof">Key for</label>
      <span class="ctl">
        <Select id="{uid}-keyof" size="sm" value={o.keyOf ?? ""} options={fillOptions(outputs, o.id)} onchange={(v: string) => patch({ keyOf: v })} />
      </span>
    {:else}
      <span class="lbl">Background</span>
      <span class="ctl">
        <Segmented
          label="Background of {o.name}"
          options={BACKGROUNDS}
          value={o.background.kind}
          onchange={(k: BackgroundKind) => patch({ background: withBackground(o, k) })}
        />
        {#if o.background.kind === "color"}
          <span class="swatch vf vf-within">
            <input
              type="color"
              aria-label="Background color of {o.name}"
              value={o.background.color}
              onchange={(e) => patch({ background: { kind: "color", color: e.currentTarget.value.toLowerCase() } })}
            />
          </span>
          <span class="mono note">{o.background.color}</span>
        {/if}
      </span>
    {/if}

    {#if o.mode === "fill"}
      <label class="lbl" for="{uid}-sound">Sound</label>
      <span class="ctl">
        <Switch id="{uid}-sound" label="Sound from {o.name}" hideLabel checked={audio.enabled} onchange={(on) => patch({ audio: { ...audio, enabled: on } })} />
        {#if !mac}
          <Select
            size="sm"
            aria-label="Audio device for {o.name}"
            disabled={!audio.enabled}
            value={audio.device ?? ""}
            options={deviceOptions(audioDevices.labels, audio.device)}
            onchange={(v: string) => patch({ audio: { enabled: audio.enabled, device: v || null } })}
          />
          {#if audio.enabled && audioDevices.labels.length === 0}<span class="note">Other devices show when the app can list them.</span>{/if}
        {/if}
      </span>
    {/if}

    <label class="lbl" for="{uid}-top">Always on top</label>
    <span class="ctl"><Switch id="{uid}-top" label="Always on top" hideLabel checked={o.alwaysOnTop} onchange={(on) => patch({ alwaysOnTop: on })} /></span>

    <span class="lbl">Test pattern</span>
    <span class="ctl patterns" role="group" aria-label="Test pattern on {o.name}">
      {#each PATTERNS as p (p.value)}
        <Button size="sm" aria-pressed={status?.pattern === p.value} onclick={() => onpattern(status?.pattern === p.value ? null : p.value)}>
          {p.label}
        </Button>
      {/each}
      {#if status?.pattern}
        <Button size="sm" variant="ghost" onclick={() => onpattern(null)}>Back to screen</Button>
      {/if}
    </span>
  </div>
</li>

<style>
  .output {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3) 0 var(--s-4);
    border-top: 1px solid var(--line);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-width: 0;
  }
  .field {
    display: inline-flex;
    border-radius: var(--r-control);
  }
  .field input {
    width: 14rem;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-weight: 500;
  }
  .status {
    display: inline-flex;
    flex: 1;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .status.muted,
  .note,
  .inherited {
    color: var(--text-muted);
  }
  .status.warning,
  .note.warn {
    color: var(--warning);
  }
  .note {
    font-size: var(--fs-sm);
  }
  .fields {
    display: grid;
    grid-template-columns: 8rem minmax(0, 1fr);
    align-items: center;
    gap: var(--s-2) var(--s-3);
    /* Under the name, past the switch. */
    padding-left: calc(32px + var(--s-3));
  }
  .lbl {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .fields > :global(.wrap) {
    justify-self: start;
    min-width: 16rem;
    max-width: 100%;
  }
  .inherited {
    font-size: var(--fs-sm);
  }
  .ctl {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .ctl > :global(.wrap) {
    min-width: 12rem;
  }
  .patterns :global([aria-pressed="true"]) {
    box-shadow: inset 0 0 0 1px var(--accent);
    color: var(--accent-text);
  }
  .swatch {
    --vf-radius: var(--r-control);
    display: inline-flex;
    width: var(--control-h-sm);
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .swatch input {
    width: 100%;
    height: 100%;
    padding: 2px;
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    cursor: pointer;
  }
  @container (max-width: 560px) {
    .fields {
      grid-template-columns: minmax(0, 1fr);
      padding-left: 0;
    }
  }
</style>
