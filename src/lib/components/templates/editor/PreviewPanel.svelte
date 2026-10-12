<!--
  Right column of the template editor: the live preview of `/screen/__preview` (one iframe, via
  LivePreview), Play in / Play out with the on-air tally, preview values (preview text only, never
  saved; the shared FieldInputs: a list edits its rows in a table, a timer runs locally, a clock
  shows its format) and the auto-out setting. Presentational: the editor owns the state and the API calls.

  Props:
    fields: FieldDef[]
    previewValues: Record<string, string>   effective preview text per field key
    onAir: boolean
    busy: boolean                            a play request is in flight
    autoOutMs: number | null
    onplayin, onplayout: () => void
    onpreviewvalue: (key: string, value: string) => void
    onautoout: (ms: number | null) => void
-->
<script lang="ts">
  import Play from "lucide-svelte/icons/play";
  import Square from "lucide-svelte/icons/square";
  import Button from "$lib/components/Button.svelte";
  import LivePreview from "$lib/components/LivePreview.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import FieldInputs from "$lib/components/fields/FieldInputs.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { screenUrl, server, statusText } from "$lib/stores/server.svelte";
  import { PREVIEW_SCREEN_ID, type FieldDef } from "$lib/types";
  import { AUTO_OUT_DEFAULT_S, AUTO_OUT_MAX_S } from "./model";

  let {
    fields,
    previewValues,
    onAir,
    busy,
    autoOutMs,
    onplayin,
    onplayout,
    onpreviewvalue,
    onautoout,
  }: {
    fields: readonly FieldDef[];
    previewValues: Record<string, string>;
    onAir: boolean;
    busy: boolean;
    autoOutMs: number | null;
    onplayin: () => void;
    onplayout: () => void;
    onpreviewvalue: (key: string, value: string) => void;
    onautoout: (ms: number | null) => void;
  } = $props();

  const uid = $props.id();
  /** One input per key: while editing, two fields can briefly share a key. */
  const uniqueFields = $derived(fields.filter((f, i) => fields.findIndex((g) => g.key === f.key) === i));

  $effect(() => server.watch());
  const status = $derived(server.status);
  const url = $derived(screenUrl(status, PREVIEW_SCREEN_ID));
  const running = $derived(url !== null);

  /** Seconds shown in the field; remembered while auto-out is off. */
  let seconds = $state(AUTO_OUT_DEFAULT_S);
  $effect(() => {
    if (autoOutMs !== null) seconds = Math.max(1, Math.round(autoOutMs / 1000));
  });
</script>

<div class="panel">
  <section class="block" aria-labelledby="{uid}-preview">
    <h2 class="section-label" id="{uid}-preview">Preview</h2>
    <LivePreview src={url} title="Template preview" selected={onAir}>
      <span class:warn={status?.state === "error"}>
        {statusText(status) ?? "The output server isn't running."}
      </span>
    </LivePreview>

    <div class="play">
      <Button variant="primary" disabled={!running || busy || onAir} onclick={onplayin}>
        {#snippet icon()}<Play size={14} />{/snippet}
        Play in
      </Button>
      <Button disabled={!running || busy || !onAir} onclick={onplayout}>
        {#snippet icon()}<Square size={14} />{/snippet}
        Play out
      </Button>
      <span class="air" aria-live="polite">
        {#if onAir}
          <TallyDot state="program" />
          <span>On air</span>
        {:else}
          <TallyDot state="idle" />
          <span class="muted">Off air</span>
        {/if}
      </span>
    </div>
  </section>

  <section class="block" aria-labelledby="{uid}-values">
    <h2 class="section-label" id="{uid}-values">Preview values</h2>
    {#if fields.length === 0}
      <p class="muted small">Add fields on the Fields tab to preview their text here.</p>
    {:else}
      <div class="values">
        <FieldInputs defs={uniqueFields} values={previewValues} onfield={onpreviewvalue} />
      </div>
    {/if}
  </section>

  <section class="block" aria-labelledby="{uid}-auto">
    <h2 class="section-label" id="{uid}-auto">Auto-out</h2>
    <div class="auto">
      <Switch
        label="Play out automatically"
        checked={autoOutMs !== null}
        onchange={(on) => onautoout(on ? seconds * 1000 : null)}
      />
      <NumberInput
        aria-label="Auto-out after seconds"
        value={seconds}
        min={1}
        max={AUTO_OUT_MAX_S}
        digits={3}
        suffix="s"
        disabled={autoOutMs === null}
        onchange={(n) => {
          seconds = n;
          onautoout(n * 1000);
        }}
      />
    </div>
  </section>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-4);
  }
  .block + .block {
    border-top: 1px solid var(--line);
  }
  .play {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .air {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    margin-left: auto;
    font-weight: 500;
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: var(--fs-sm);
  }
  .warn {
    color: var(--warning);
  }
  .values {
    font-size: var(--fs-sm);
  }
  .auto {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
  }
</style>
