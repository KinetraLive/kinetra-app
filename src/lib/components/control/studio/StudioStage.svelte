<!--
  StudioStage: Control's single view in Studio mode (lazy studio chunk), a multiview sized to the
  stage, with a screen picker above it (the screen list is hidden in Studio mode). Preview on the left (the
  REAL renderer at /screen/<id>/preview, green tally border), Program on the right (/screen/<id>,
  red tally border), each labelled with a TallyDot and text. Under Preview: Play in / Play out for
  every layer it shows (Preview only, nothing reaches Program); its header says what a take would
  do ("2 in · 1 out · 1 updated", or "same as Program"). The centre column:
    TAKE      Preview → Program with the transition below (also Ctrl+Enter): Cut is instant, Auto
              plays each graphic's own out / in animations, Crossfade runs over its duration.
              Disabled while the take would change nothing on air
    CUT       the same take as a cut (also Ctrl+Shift+Enter)
    transition Cut / Auto / Crossfade and the crossfade duration (ms, mono): the
              project's saved default (set_studio), the same one the settings popover edits
    T-bar     the same take by hand (see TBar)
  Exactly two iframes, both through LivePreview (its visibility rules apply to each).

  Props:
    ctl: ControlCtl
    view: LiveScreenView    the selected screen
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import LivePreview from "$lib/components/LivePreview.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { toAppError } from "$lib/api";
  import { studioOp } from "$lib/api/live";
  import { keysFor } from "$lib/commands/bindings.svelte";
  import { previews } from "$lib/stores/preview.svelte";
  import { screenUrl, server } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { LiveScreenView } from "$lib/types";
  import type { ControlCtl } from "../control.svelte";
  import Segmented from "../Segmented.svelte";
  import { rehearse, summaryText, takeSummary } from "./ops";
  import TBar from "./TBar.svelte";
  import { TbarOps } from "./tbar";
  import { TAKE_KINDS, TAKE_MS, clampMs, transitionText } from "./transitions";

  let { ctl, view }: { ctl: ControlCtl; view: LiveScreenView } = $props();

  const studio = $derived(ctl.studio);
  const id = $derived(view.screen.id);
  const pv = $derived(previews.views.get(id));
  const changes = $derived(pv?.cues.length ?? 0);
  /** What a take would do on air ("" = nothing). */
  const summary = $derived(summaryText(takeSummary(pv, view.layers)));
  /** The layers Preview shows (Play in / Play out play them all); media layers have no animation to rehearse. */
  const shown = $derived((pv?.layers ?? view.layers).filter((l) => l.onAir && !l.media).map((l) => l.id));
  const takeKey = $derived(keysFor("studio.take")[0] ?? "");
  const cutKey = $derived(keysFor("studio.cut")[0] ?? "");
  const running = $derived(server.status?.state === "running");
  const program = $derived(screenUrl(server.status, id));
  const preview = $derived(program && `${program}/preview`);
  const t = $derived(studio.transition);

  let busy = $state(false);

  // One T-bar op queue per screen; a screen switch mid-drag aborts the old one.
  const ops = $derived.by(() => {
    const screenId = id;
    return new TbarOps((op) =>
      studioOp(screenId, op).then(
        (v) => previews.put(v),
        (e) => toasts.push({ kind: "error", message: `T-bar: ${toAppError(e).message}` }),
      ),
    );
  });
  $effect(() => {
    const o = ops;
    return () => o.dispose();
  });

  async function take(cut = false) {
    if (busy) return;
    busy = true;
    await studio.take(id, cut ? { kind: "cut", ms: t.ms } : null);
    busy = false;
  }
  const how = $derived(t.kind === "cut" ? "Cut · instant" : transitionText(t));
</script>

{#snippet stopped()}Server not running{/snippet}

<div class="wrap">
  <div class="top">
    <Select
      size="sm"
      aria-label="Screen"
      value={id}
      options={ctl.visible.map((v) => ({ value: v.screen.id, label: v.screen.name }))}
      onchange={ctl.select}
    />
    <span class="res mono">{view.screen.width}×{view.screen.height}</span>
  </div>
  <div class="studio" style:--ar={view.screen.width / view.screen.height}>
    <section class="bus pvw" aria-label="Preview">
      <header class="label">
        <TallyDot state="preview" />
        <span class="name">Preview</span>
        <span class="meta">{changes === 0 ? "same as Program" : summary || "no change on air"}</span>
      </header>
      <LivePreview
        src={preview}
        title="{view.screen.name} Preview"
        width={view.screen.width}
        height={view.screen.height}
        children={running ? undefined : stopped}
      />
      <div class="rehearse" role="group" aria-label="Rehearse in Preview">
        <Button size="sm" disabled={!running || shown.length === 0} onclick={() => rehearse(id, shown, "in")}>Play in</Button>
        <Button size="sm" disabled={!running || shown.length === 0} onclick={() => rehearse(id, shown, "out")}>Play out</Button>
        <span class="hint">Preview only</span>
      </div>
    </section>

    <div class="centre" role="group" aria-label="Take">
      <div class="ctl">
        <button
          type="button"
          class="take vf"
          disabled={busy || !summary}
          aria-keyshortcuts={takeKey ? takeKey.replace("Ctrl", "Control") : undefined}
          title={summary ? `Preview to Program (${how})${takeKey ? ` · ${takeKey}` : ""}` : "Nothing to take"}
          onclick={() => take()}
        >
          TAKE
        </button>
        <span class="caption">{how}</span>
        <Button
          size="sm"
          disabled={busy || !summary}
          title={summary ? `Preview to Program as a cut${cutKey ? ` · ${cutKey}` : ""}` : "Nothing to take"}
          onclick={() => take(true)}
        >
          CUT
        </Button>
        <div class="kinds">
          <Segmented
            label="Transition"
            options={TAKE_KINDS}
            value={t.kind}
            onchange={(kind) => studio.setTransition({ ...t, kind })}
          />
        </div>
        <NumberInput
          aria-label="Crossfade duration"
          value={t.ms}
          min={TAKE_MS.min}
          max={TAKE_MS.max}
          step={50}
          digits={4}
          suffix="ms"
          disabled={t.kind !== "crossfade"}
          onchange={(ms) => studio.setTransition({ ...t, ms: clampMs(ms) })}
        />
      </div>
      <div class="tb"><TBar value={pv?.tbar ?? null} {ops} /></div>
    </div>

    <section class="bus pgm" aria-label="Program">
      <header class="label">
        <TallyDot state="program" />
        <span class="name">Program</span>
        <span class="meta">{view.layers.filter((l) => l.onAir).length} on air</span>
      </header>
      <LivePreview
        src={program}
        title="{view.screen.name} Program"
        width={view.screen.width}
        height={view.screen.height}
        children={running ? undefined : stopped}
      />
    </section>
  </div>
</div>

<style>
  /* A multiview: both monitors as large as the stage allows (width from the container, height
     from the viewport minus the bars above), the centre column between them, all centred. Pure
     CSS: no measuring. */
  .wrap {
    container-type: inline-size;
    display: grid;
    grid-template-rows: auto 1fr;
    gap: var(--s-3);
    min-height: 100%;
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .res {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .studio {
    --col: 10rem;
    --gap: var(--s-3);
    /* Room for a monitor's picture: the viewport minus the top bar, toolbar, padding, the screen
       row and the monitor's own label and frame. */
    --h: max(120px, calc(100dvh - var(--topbar-h) - 160px));
    display: grid;
    grid-template-columns: auto var(--col) auto;
    align-self: center;
    justify-content: center;
    align-items: center;
    gap: var(--gap);
  }
  .bus {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    /* Picture width: half the row (minus the column and the frame) or the height bound × aspect;
       20px = padding + border on both sides. */
    width: calc(min((100cqw - var(--col) - 2 * var(--gap)) / 2 - 20px, var(--h) * var(--ar)) + 20px);
    min-width: 0;
    padding: var(--s-2);
    border: 2px solid var(--bus);
    border-radius: var(--r-panel);
  }
  .pvw {
    --bus: var(--tally-preview);
  }
  .pgm {
    --bus: var(--tally-program);
  }
  .label {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .name {
    font-family: var(--font-heading);
    font-size: var(--fs-label);
    font-weight: 600;
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
  }
  .meta {
    margin-left: auto;
    overflow: hidden;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* As tall as the monitors: the controls, and beside them the T-bar at full height. */
  .centre {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--s-2);
    align-self: stretch;
    min-height: 0;
  }
  .ctl {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .tb {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--s-1);
    min-height: 0;
  }
  .take {
    --vf-radius: var(--r-control);
    border-radius: var(--r-control);
    font-weight: 600;
  }
  .take {
    height: 48px;
    background: var(--accent);
    color: var(--accent-ink);
    font-family: var(--font-heading);
    font-size: var(--fs-xl);
    letter-spacing: var(--tracking-label);
  }
  .take:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .take:disabled {
    opacity: 0.6;
  }
  .rehearse {
    display: flex;
    align-items: center;
    gap: var(--s-1);
  }
  .hint {
    margin-left: auto;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .caption {
    margin-top: calc(-1 * var(--s-1));
    color: var(--text-muted);
    font-size: var(--fs-xs);
    text-align: center;
  }
  /* The segmented control stacked: one option per row in the narrow column. */
  .kinds :global(.seg) {
    display: flex;
    flex-direction: column;
    width: 100%;
  }
  .kinds :global(.opt) {
    padding: 0 var(--s-2);
  }
  .tb :global(.tbar) {
    width: 32px;
  }
  @container (max-width: 640px) {
    .studio {
      grid-template-columns: minmax(0, 1fr);
    }
    .bus {
      width: 100%;
    }
    .centre {
      order: 1;
    }
    .pgm {
      order: 2;
    }
  }
</style>
