<!--
  FlowRows: one group of the /flows list. Each row: the armed switch (optimistic), the name (a link
  to the editor; Rename turns it into an input: Enter saves, Escape cancels), the trigger summary,
  notes (Running, "Not deployed" with a dot, the warnings count with text and the messages in the
  tooltip, an invalid file's error, Read-only) and when it last fired (mono, relative, formatted on
  render: no timers), then Rename, Duplicate and Delete… (asks first).

  Keyboard: the names are links in a list; ArrowUp / ArrowDown move between rows (across groups).

  Props:
    flows: FlowInfo[]
    label: string                accessible name of the list
    summarize: ((f) => string) | null   the trigger summary (a lazy chunk; blank until loaded)
    renaming: string | null      the flow being renamed
    onrename: (id, name) => Promise<boolean>
    onstartrename / oncancelrename
    onarm: (id, armed) => void
    onduplicate: (id) => void
    ondelete: (flow) => void
-->
<script lang="ts">
  import { tick } from "svelte";
  import Copy from "lucide-svelte/icons/copy";
  import Pencil from "lucide-svelte/icons/pencil";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import Switch from "$lib/components/Switch.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import { FLOW_NAME_MAX, type FlowInfo } from "$lib/types";
  import { nameError } from "../names";
  import { relTime } from "../common";
  import { invalidOf } from "./groups";

  let {
    flows,
    label,
    summarize,
    renaming,
    onrename,
    onstartrename,
    oncancelrename,
    onarm,
    onduplicate,
    ondelete,
  }: {
    flows: FlowInfo[];
    label: string;
    summarize: ((f: FlowInfo) => string) | null;
    renaming: string | null;
    onrename: (id: string, name: string) => Promise<boolean>;
    onstartrename: (id: string) => void;
    oncancelrename: () => void;
    onarm: (id: string, armed: boolean) => void;
    onduplicate: (id: string) => void;
    ondelete: (f: FlowInfo) => void;
  } = $props();

  let draft = $state("");
  let error = $state<string | null>(null);

  $effect(() => {
    const id = renaming;
    const f = flows.find((x) => x.id === id);
    if (!f) return;
    draft = f.name;
    error = null;
    void tick().then(() => {
      const input = document.querySelector<HTMLInputElement>(`[data-flow="${CSS.escape(f.id)}"] input.rename`);
      input?.focus();
      input?.select();
    });
  });

  async function commit(f: FlowInfo) {
    if (renaming !== f.id) return;
    const next = draft.trim();
    if (next === f.name) return oncancelrename();
    error = nameError(next);
    if (error) return;
    if (await onrename(f.id, next)) refocus(f.id);
  }

  function refocus(id: string) {
    void tick().then(() => document.querySelector<HTMLElement>(`[data-flow="${CSS.escape(id)}"] a.name`)?.focus());
  }

  function onRenameKey(e: KeyboardEvent, f: FlowInfo) {
    if (e.key === "Enter") {
      e.preventDefault();
      void commit(f);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      oncancelrename();
      refocus(f.id);
    }
  }

  function onLinkKey(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const all = [...document.querySelectorAll<HTMLElement>("a.name[data-row-link]")];
    const i = all.indexOf(e.currentTarget as HTMLElement);
    const next = all[i + (e.key === "ArrowDown" ? 1 : -1)];
    if (!next) return;
    e.preventDefault();
    next.focus();
  }
</script>

<ul class="rows" aria-label={label}>
  {#each flows as f (f.id)}
    {@const invalid = invalidOf(f)}
    {@const warnings = f.warnings.filter((w) => w !== invalid)}
    <li class="row" data-flow={f.id}>
      <Switch label="Arm {f.name}" hideLabel checked={f.armed} disabled={f.readOnly} onchange={(on) => onarm(f.id, on)} />
      <span class="text">
        {#if renaming === f.id}
          <span class="rename-wrap vf vf-within">
            <input
              class="rename"
              type="text"
              aria-label="Name of {f.name}"
              maxlength={FLOW_NAME_MAX + 20}
              autocomplete="off"
              bind:value={draft}
              aria-invalid={error ? "true" : undefined}
              onkeydown={(e) => onRenameKey(e, f)}
              onblur={() => void commit(f)}
            />
          </span>
          {#if error}<span class="bad">{error}</span>{/if}
        {:else}
          <a class="name vf" class:off={!f.armed} href="/flows/{f.id}" data-row-link onkeydown={onLinkKey}>{f.name}</a>
        {/if}
        <span class="sum">{summarize ? summarize(f) : " "}</span>
        {#if f.running || f.draft || f.warnings.length > 0 || f.readOnly}
          <span class="notes">
            {#if f.running}<span class="running">Running</span>{/if}
            {#if f.draft}
              <span
                class="undeployed"
                class:armed={f.armed}
                title={f.armed ? "Armed: it runs the last deployed version until you deploy the changes" : undefined}
                ><span class="dot" aria-hidden="true"></span>{f.armed ? "Not deployed: runs the old version" : "Not deployed"}</span
              >
            {/if}
            {#if invalid}<span class="bad">{invalid.message}</span>{/if}
            {#if warnings.length > 0}
              <span class="warn" title={warnings.map((w) => w.message).join("\n")}>
                <TriangleAlert size={12} aria-hidden="true" />
                {warnings.length} {warnings.length === 1 ? "warning" : "warnings"}
              </span>
            {/if}
            {#if f.readOnly}<span class="muted">Read-only (from a newer version)</span>{/if}
          </span>
        {/if}
      </span>
      <span class="fired" title={f.lastFiredAt ? `Last fired ${new Date(f.lastFiredAt).toLocaleString()}` : undefined}>
        {f.lastFiredAt ? relTime(f.lastFiredAt) : "never"}
      </span>
      <span class="tools">
        <IconButton size="sm" label="Rename {f.name}" disabled={f.readOnly} onclick={() => onstartrename(f.id)}>
          <Pencil size={14} />
        </IconButton>
        <IconButton size="sm" label="Duplicate {f.name}" onclick={() => onduplicate(f.id)}>
          <Copy size={14} />
        </IconButton>
        <IconButton variant="danger" size="sm" label="Delete {f.name}…" tooltip="left" disabled={f.readOnly} onclick={() => ondelete(f)}>
          <Trash2 size={14} />
        </IconButton>
      </span>
    </li>
  {/each}
</ul>

<style>
  .rows {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    list-style: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-height: calc(var(--row-h) + var(--s-3));
    padding: var(--s-2) var(--s-3);
  }
  .row + .row {
    border-top: 1px solid var(--line);
  }
  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    line-height: 1.3;
  }
  .name {
    --vf-radius: 4px;
    align-self: flex-start;
    max-width: 100%;
    overflow: hidden;
    color: var(--text);
    font-weight: 500;
    text-decoration: none;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name:hover {
    text-decoration: underline;
  }
  .name.off {
    color: var(--text-muted);
  }
  .sum {
    overflow: hidden;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .notes {
    display: flex;
    flex-wrap: wrap;
    gap: 0 var(--s-3);
    font-size: var(--fs-xs);
  }
  .running {
    color: var(--accent-text);
  }
  .undeployed {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .undeployed.armed {
    color: var(--warning);
  }
  .undeployed.armed .dot {
    background: var(--warning);
  }
  .warn {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--warning);
  }
  .bad {
    color: var(--danger);
    font-size: var(--fs-xs);
  }
  .muted,
  .fired {
    color: var(--text-muted);
  }
  .fired {
    flex: none;
    font-variant-numeric: tabular-nums;
    font-size: var(--fs-sm);
  }
  .tools {
    display: inline-flex;
    flex: none;
  }
  .rename-wrap {
    --vf-radius: var(--r-control);
    display: flex;
    max-width: 28rem;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .rename {
    flex: 1;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
    color: var(--text);
  }
</style>
