<!--
  LogicForm: the inspector's config form for logic nodes (lazy chunk). Branch (nothing to set),
  Switch (its cases, up to 16; one output each, plus default), Gate (starts open or closed), Delay,
  Wait until (a time of day or a release; optional timeout), Debounce, Throttle, Sequence and Random
  pick (2 to 8 outputs), Once, Counter (every N events), Repeat (1 to 10 000 rounds or Forever, and
  a wait between rounds).

  Removing a case or an output drops the wires on it.

  Props: FormProps (../load.ts).
-->
<script lang="ts">
  import Plus from "lucide-svelte/icons/plus";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { FLOW_BRANCHES_MAX, FLOW_CASES_MAX, FLOW_DELAY_MAX_MS } from "$lib/types";
  import type { FormProps } from "../load";
  import Field from "./Field.svelte";
  import { commit, toMs, toSeconds } from "./form";

  let { ed, node }: FormProps = $props();
  const ro = $derived(ed.readOnly);
  const MAX_S = FLOW_DELAY_MAX_MS / 1000;
  /** Repeat: rounds 1..=10 000; switching Forever off starts again at 3. */
  const REPEAT_MAX = 10_000;
  const REPEAT_DEFAULT = 3;
</script>

{#snippet seconds(label: string, ms: number, min: number, set: (ms: number) => void)}
  <Field {label}>
    <NumberInput aria-label={label} disabled={ro} value={toSeconds(ms)} min={min} max={MAX_S} step={0.1} digits={6} suffix="s" onchange={(v) => set(toMs(v))} />
  </Field>
{/snippet}

{#if node.type === "logic.switch"}
  {@const c = node.config}
  <Field label="Cases (● value is compared as text)">
    <ol class="cases">
      {#each c.cases as v, i (i)}
        <li class="case">
          <span class="num">{i + 1}</span>
          <span class="text vf vf-within">
            <input
              type="text"
              aria-label="Case {i + 1}"
              autocomplete="off"
              disabled={ro}
              value={v}
              oninput={(e) => commit(ed, node, { cases: c.cases.map((x, j) => (j === i ? e.currentTarget.value : x)) }, `case${i}`)}
            />
          </span>
          <IconButton variant="danger"
            size="sm"
            label="Remove case {i + 1}"
            tooltip="left"
            disabled={ro || c.cases.length <= 1}
            onclick={() => commit(ed, node, { cases: c.cases.filter((_, j) => j !== i) })}
          >
            <Trash2 size={14} />
          </IconButton>
        </li>
      {/each}
    </ol>
    <Button size="sm" variant="ghost" disabled={ro || c.cases.length >= FLOW_CASES_MAX} onclick={() => commit(ed, node, { cases: [...c.cases, ""] })}>
      {#snippet icon()}<Plus size={14} />{/snippet}
      Add case
    </Button>
  </Field>
{:else if node.type === "logic.gate"}
  {@const c = node.config}
  <Switch label="Starts open" disabled={ro} checked={c.open} onchange={(on) => commit(ed, node, { open: on })} />
{:else if node.type === "logic.delay"}
  {@render seconds("Wait (a wired ● ms replaces it)", node.config.ms, 0, (ms) => commit(ed, node, { ms }))}
{:else if node.type === "logic.waitUntil"}
  {@const c = node.config}
  <Field
    label="Until a time of day (empty: until ▶ release)"
    input={{ value: c.at ?? "", type: "time", optional: true }}
    disabled={ro}
    oninput={(v) => commit(ed, node, { ...c, at: v === "" ? null : v }, "at")}
  />
  <Field label="Timeout (0: none)">
    <NumberInput
      aria-label="Timeout"
      disabled={ro}
      value={c.timeoutMs ? toSeconds(c.timeoutMs) : 0}
      min={0}
      max={MAX_S}
      step={0.1}
      digits={6}
      suffix="s"
      onchange={(v) => commit(ed, node, { ...c, timeoutMs: v > 0 ? toMs(v) : null })}
    />
  </Field>
{:else if node.type === "logic.debounce"}
  {@render seconds("Quiet period", node.config.ms, 0.001, (ms) => commit(ed, node, { ms }))}
{:else if node.type === "logic.throttle"}
  {@render seconds("At most once per", node.config.ms, 0.001, (ms) => commit(ed, node, { ms }))}
{:else if node.type === "logic.sequence"}
  <Field label="Outputs">
    <NumberInput aria-label="Outputs" disabled={ro} value={node.config.count} min={2} max={FLOW_BRANCHES_MAX} digits={2} onchange={(v) => commit(ed, node, { count: v })} />
  </Field>
{:else if node.type === "logic.random"}
  {@const c = node.config}
  <Field label="Outputs">
    <NumberInput aria-label="Outputs" disabled={ro} value={c.count} min={2} max={FLOW_BRANCHES_MAX} digits={2} onchange={(v) => commit(ed, node, { ...c, count: v })} />
  </Field>
  <Switch label="Never the same twice in a row" disabled={ro} checked={c.noRepeat} onchange={(on) => commit(ed, node, { ...c, noRepeat: on })} />
{:else if node.type === "logic.repeat"}
  {@const c = node.config}
  <Field label="Rounds">
    {#if c.count !== null}
      <NumberInput aria-label="Rounds" disabled={ro} value={c.count} min={1} max={REPEAT_MAX} digits={5} onchange={(v) => commit(ed, node, { ...c, count: v })} />
    {/if}
    <Switch label="Forever" disabled={ro} checked={c.count === null} onchange={(on) => commit(ed, node, { ...c, count: on ? null : REPEAT_DEFAULT })} />
  </Field>
  {@render seconds("Wait between rounds", c.delayMs, 0, (ms) => commit(ed, node, { ...c, delayMs: ms }))}
  <p class="help">Each round waits for the body to finish. Rounds start at most every 100 ms.</p>
{:else if node.type === "logic.counter"}
  <Field label="Every N events">
    <NumberInput aria-label="Every N events" disabled={ro} value={node.config.every} min={1} max={100000} digits={6} onchange={(v) => commit(ed, node, { every: v })} />
  </Field>
{/if}

<style>
  .help {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .cases {
    display: flex;
    flex: 1 1 100%;
    flex-direction: column;
    gap: var(--s-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .case {
    display: flex;
    align-items: center;
    gap: var(--s-1);
  }
  .num {
    width: 2ch;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .text {
    --vf-radius: var(--r-control);
    display: flex;
    flex: 1;
    min-width: 0;
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .text input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
    color: var(--text);
  }
</style>
