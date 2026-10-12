<!--
  Variables tab. Two parts:
    1. Values: the no-code form, the shared VariableValuesForm (also used by the screen editor
       inspector). The values are the active variant's; a change sets that variant's value
       (`onvalue`).
    2. Definitions ("Edit variables", collapsible): add, delete, reorder; key, label, kind and the
       kind's settings (range min/max/step/unit, select options) (`ondefs`).

  Props:
    variables: VariableDef[]
    otherKeys: string[]          field keys (new variable keys avoid them)
    issues: Issue[]
    onvalue: (index: number, value: string) => void
    ondefs: (next: VariableDef[]) => void
-->
<script lang="ts">
  import { tick } from "svelte";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import Plus from "lucide-svelte/icons/plus";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import X from "lucide-svelte/icons/x";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import VariableValuesForm from "$lib/components/variables/VariableValuesForm.svelte";
  import {
    TEMPLATE_VARIABLES_MAX,
    type SelectOption,
    type VariableDef,
    type VariableKind,
  } from "$lib/types";
  import {
    VARIABLE_KINDS,
    issueFor,
    newVariable,
    removeAt,
    replaceAt,
    swap,
    withKind,
    type Issue,
  } from "./model";

  let {
    variables,
    otherKeys,
    issues,
    onvalue,
    ondefs,
  }: {
    variables: readonly VariableDef[];
    otherKeys: readonly string[];
    issues: readonly Issue[];
    onvalue: (index: number, value: string) => void;
    ondefs: (next: VariableDef[]) => void;
  } = $props();

  const uid = $props.id();
  let defsOpen = $state(false);
  let defsList: HTMLDivElement | undefined = $state();
  const full = $derived(variables.length >= TEMPLATE_VARIABLES_MAX);
  const defsHaveIssues = $derived(
    issues.some((i) => i.scope === "variable" && i.prop !== "value"),
  );

  // Open the definitions when they hold an error, so the message is visible.
  $effect(() => {
    if (defsHaveIssues) defsOpen = true;
  });

  function patch(i: number, p: Partial<VariableDef>) {
    ondefs(replaceAt(variables, i, { ...variables[i], ...p } as VariableDef));
  }

  function setKind(i: number, type: VariableKind["type"]) {
    const v = variables[i];
    if (v.type === type) return;
    ondefs(replaceAt(variables, i, withKind(v, type)));
  }

  function setOptions(i: number, options: SelectOption[]) {
    const v = variables[i];
    if (v.type !== "select") return;
    // Keep the default pointing at an existing option.
    const def = options.some((o) => o.value === v.default) ? v.default : (options[0]?.value ?? "");
    ondefs(replaceAt(variables, i, { ...v, options, default: def }));
  }

  async function add() {
    if (full) return;
    defsOpen = true;
    ondefs([...variables, newVariable([...variables.map((v) => v.key), ...otherKeys])]);
    await tick();
    defsList
      ?.querySelector<HTMLInputElement>(`[data-def="${variables.length - 1}"] input`)
      ?.focus();
  }

  async function move(i: number, dir: -1 | 1, what: string) {
    ondefs(swap(variables, i, i + dir));
    await tick();
    const b = defsList?.querySelector<HTMLButtonElement>(`[data-def="${i + dir}"] [data-act="${what}"]`);
    if (b && !b.disabled) b.focus();
  }

  const valueErrors = $derived(variables.map((_, i) => issueFor(issues, "variable", i, "value")));
</script>

<div class="vars">
  <!-- 1. Values -->
  <section class="values" aria-labelledby="{uid}-values">
    <h2 class="section-label" id="{uid}-values">Values</h2>
    {#if variables.length === 0}
      <p class="hint">
        Colors, fonts and sizes you can change without code. In CSS:
        <code>var(--&lt;key&gt;)</code>.
      </p>
    {:else}
      <VariableValuesForm
        {variables}
        errors={valueErrors}
        onvalue={(_key, value, i) => onvalue(i, value)}
      />
    {/if}
  </section>

  <!-- 2. Definitions -->
  <section class="defs" aria-labelledby="{uid}-defs-btn">
    <button
      type="button"
      class="disclosure vf"
      id="{uid}-defs-btn"
      aria-expanded={defsOpen}
      aria-controls="{uid}-defs"
      onclick={() => (defsOpen = !defsOpen)}
    >
      <span class="chev" class:open={defsOpen} aria-hidden="true"><ChevronRight size={14} /></span>
      Edit variables
      <span class="count">{variables.length} / {TEMPLATE_VARIABLES_MAX}</span>
    </button>

    <div class="defs-body" id="{uid}-defs" hidden={!defsOpen}>
      <div class="def-list" bind:this={defsList}>
        {#each variables as v, i}
          {@const keyErr = issueFor(issues, "variable", i, "key")}
          {@const labelErr = issueFor(issues, "variable", i, "label")}
          {@const rangeErr = issueFor(issues, "variable", i, "range")}
          {@const optErr = issueFor(issues, "variable", i, "options")}
          <div class="def" data-def={i}>
            <div class="def-main">
              <span class="cell vf vf-within">
                <input
                  class="mono"
                  type="text"
                  aria-label="Variable {i + 1} key"
                  spellcheck="false"
                  autocomplete="off"
                  value={v.key}
                  aria-invalid={keyErr ? "true" : undefined}
                  oninput={(e) => patch(i, { key: e.currentTarget.value })}
                />
              </span>
              <span class="cell vf vf-within">
                <input
                  type="text"
                  aria-label="Variable {i + 1} label"
                  autocomplete="off"
                  value={v.label}
                  aria-invalid={labelErr ? "true" : undefined}
                  oninput={(e) => patch(i, { label: e.currentTarget.value })}
                />
              </span>
              <Select
                class="kind"
                aria-label="Variable {i + 1} kind"
                value={v.type}
                options={VARIABLE_KINDS}
                onchange={(type: VariableKind["type"]) => setKind(i, type)}
              />
              <span class="acts">
                <IconButton size="sm" label="Move up" tooltip="top" data-act="up" disabled={i === 0} onclick={() => move(i, -1, "up")}>
                  <ArrowUp size={14} />
                </IconButton>
                <IconButton
                  size="sm"
                  label="Move down"
                  tooltip="top"
                  data-act="down"
                  disabled={i === variables.length - 1}
                  onclick={() => move(i, 1, "down")}
                >
                  <ArrowDown size={14} />
                </IconButton>
                <IconButton variant="danger" size="sm" label="Delete variable" tooltip="top" data-act="delete" onclick={() => ondefs(removeAt(variables, i))}>
                  <Trash2 size={14} />
                </IconButton>
              </span>
            </div>

            {#if v.type === "range"}
              <div class="settings">
                <label class="setting">
                  <span>Min</span>
                  <NumberInput value={v.min} digits={5} step={0.01} onchange={(n) => patch(i, { min: n })} />
                </label>
                <label class="setting">
                  <span>Max</span>
                  <NumberInput value={v.max} digits={5} step={0.01} onchange={(n) => patch(i, { max: n })} />
                </label>
                <label class="setting">
                  <span>Step</span>
                  <NumberInput value={v.step} digits={5} min={0} step={0.01} onchange={(n) => patch(i, { step: n })} />
                </label>
                <label class="setting">
                  <span>Unit</span>
                  <span class="cell unit vf vf-within">
                    <input
                      class="mono"
                      type="text"
                      spellcheck="false"
                      autocomplete="off"
                      maxlength="8"
                      value={v.unit}
                      oninput={(e) => patch(i, { unit: e.currentTarget.value.trim() })}
                    />
                  </span>
                </label>
              </div>
            {:else if v.type === "select"}
              <div class="options">
                <span class="opt-head" aria-hidden="true"><span>Value</span><span>Label</span></span>
                {#each v.options as o, j}
                  <div class="opt">
                    <span class="cell vf vf-within">
                      <input
                        class="mono"
                        type="text"
                        aria-label="Variable {i + 1} option {j + 1} value"
                        spellcheck="false"
                        autocomplete="off"
                        value={o.value}
                        oninput={(e) =>
                          setOptions(i, replaceAt(v.options, j, { ...o, value: e.currentTarget.value }))}
                      />
                    </span>
                    <span class="cell vf vf-within">
                      <input
                        type="text"
                        aria-label="Variable {i + 1} option {j + 1} label"
                        autocomplete="off"
                        value={o.label}
                        oninput={(e) =>
                          setOptions(i, replaceAt(v.options, j, { ...o, label: e.currentTarget.value }))}
                      />
                    </span>
                    <IconButton variant="danger"
                      size="sm"
                      label="Remove option"
                      tooltip="top"
                      disabled={v.options.length <= 1}
                      onclick={() => setOptions(i, removeAt(v.options, j))}
                    >
                      <X size={14} />
                    </IconButton>
                  </div>
                {/each}
                <Button
                  size="sm"
                  variant="ghost"
                  onclick={() => {
                    const n = v.options.length + 1;
                    setOptions(i, [...v.options, { value: `option-${n}`, label: `Option ${n}` }]);
                  }}
                >
                  {#snippet icon()}<Plus size={14} />{/snippet}
                  Add option
                </Button>
              </div>
            {/if}

            {#each [keyErr, labelErr, rangeErr, optErr].filter(Boolean) as msg}
              <p class="err">{msg}</p>
            {/each}
          </div>
        {/each}
      </div>
      <div class="foot">
        <Button size="sm" onclick={add} disabled={full}>
          {#snippet icon()}<Plus size={14} />{/snippet}
          Add variable
        </Button>
      </div>
    </div>
  </section>
</div>

<style>
  .vars {
    display: flex;
    flex-direction: column;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-4);
  }
  .defs {
    border-top: 1px solid var(--line);
    gap: var(--s-2);
  }
  .hint {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }

  .cell {
    display: flex;
    min-width: 0;
    border-radius: var(--r-control);
  }
  .cell input {
    width: 100%;
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }

  .disclosure {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    align-self: flex-start;
    height: var(--control-h);
    padding: 0 var(--s-2) 0 var(--s-1);
    border-radius: var(--r-control);
    color: var(--text);
    font-weight: 500;
  }
  .disclosure:hover {
    background: var(--surface-3);
  }
  .chev {
    display: inline-flex;
    color: var(--text-muted);
    transition: transform var(--dur-fast) var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .count {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .defs-body {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
  }
  .defs-body[hidden] {
    display: none;
  }
  .def-list {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .def {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .def .cell input {
    background: var(--surface-1);
  }
  .def-main {
    display: grid;
    grid-template-columns: minmax(7rem, 1fr) minmax(8rem, 1.4fr) 8rem auto;
    gap: var(--s-2);
    align-items: center;
  }
  .def-main :global(.kind) {
    width: 100%;
  }
  .acts {
    display: inline-flex;
    gap: 2px;
  }
  .settings {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-3);
  }
  .setting {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .unit {
    width: 8ch;
  }
  .options {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    align-items: flex-start;
  }
  .opt-head,
  .opt {
    display: grid;
    grid-template-columns: minmax(7rem, 12rem) minmax(8rem, 16rem) auto;
    gap: var(--s-2);
    align-items: center;
  }
  .opt-head {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  @media (max-width: 720px) {
    .def-main {
      grid-template-columns: 1fr 1fr;
    }
  }
</style>
