<!--
  FlowList: the rule rows of Control's Automation strip (lazy chunk), under an automation or
  "Other rules". Each row: the armed switch (saved at once, optimistic), the name, the one-line
  summary ("When … then …", or "N nodes" for a flow that isn't a simple chain), its running or
  failed state and when it last fired ("3 min ago", mono; formatted on render, no timers). Warnings
  are not shown per row: the strip aggregates them per automation. The row menu: Edit (simple
  chains only), Edit in Flows (/flows/<id>, every flow), Duplicate, Delete…
  (confirmed). A flow from a newer app version is read-only: no arming, editing or deleting (its
  duplicate is editable).

  Props:
    flows: FlowInfo[]
    ctl: ControlCtl
    Menu: MenuType       Control's Menu, handed down (see control/strips.ts)
    lookups: Lookups     names for the summaries
    label: string        accessible name of the list
-->
<script lang="ts">
  import Ellipsis from "lucide-svelte/icons/ellipsis";
  import { goto } from "$app/navigation";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import type { ControlCtl } from "$lib/components/control/control.svelte";
  import type { MenuItem } from "$lib/components/control/Menu.svelte";
  import type { MenuType } from "$lib/components/control/strips";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import type { FlowInfo } from "$lib/types";
  import { duplicate, remove, setArmed } from "./ops";
  import { relTime, summary, toLinear, type Lookups } from "./linear";

  let {
    flows,
    ctl,
    Menu,
    lookups,
    label,
  }: { flows: FlowInfo[]; ctl: ControlCtl; Menu: MenuType; lookups: Lookups; label: string } = $props();

  let deleting = $state<FlowInfo | null>(null);
  let busy = $state(false);

  function items(f: FlowInfo): MenuItem[] {
    const out: MenuItem[] = [];
    if (toLinear(f)) {
      out.push({
        id: "edit",
        label: "Edit",
        disabled: f.readOnly,
        onselect: () => void ctl.flowEditor.show({ mode: "edit", id: f.id }),
      });
    }
    // Simple mode hides the flow editor.
    if (isAdvanced()) out.push({ id: "open", label: "Edit in Flows", onselect: () => void goto(`/flows/${f.id}`) });
    out.push(
      { id: "duplicate", label: "Duplicate", onselect: () => void duplicate(ctl.flows, f.id) },
      { id: "delete", label: "Delete…", danger: true, disabled: f.readOnly, onselect: () => (deleting = f) },
    );
    return out;
  }

  async function confirmDelete() {
    const f = deleting;
    if (!f) return;
    busy = true;
    await remove(ctl, f.id);
    busy = false;
    deleting = null;
  }
</script>

<ul class="list" aria-label={label}>
  {#each flows as f (f.id)}
    {@const failed = ctl.flows.failed.get(f.id)}
    <li class="flow" data-flow={f.id}>
      <Switch label="Arm {f.name}" hideLabel checked={f.armed} disabled={f.readOnly} onchange={(on) => setArmed(ctl.flows, f.id, on)} />
      <span class="text">
        <span class="name" class:off={!f.armed}>{f.name}</span>
        <span class="sum">{summary(f, lookups)}</span>
        {#if f.running || failed || f.readOnly}
          <span class="notes">
            {#if f.running}<span class="running" role="status">Running</span>{/if}
            {#if failed && !f.running}
              <span class="failed" role="status">Failed{failed.message ? `: ${failed.message}` : ""}</span>
            {/if}
            {#if f.readOnly}<span class="muted">Read-only (from a newer version)</span>{/if}
          </span>
        {/if}
      </span>
      <span class="fired" title={f.lastFiredAt ?? undefined}>
        {f.lastFiredAt ? relTime(f.lastFiredAt) : "never"}
      </span>
      <Menu label="Actions for {f.name}" items={items(f)} align="end">
        {#snippet icon()}<Ellipsis size={14} />{/snippet}
      </Menu>
    </li>
  {/each}
</ul>

{#if deleting}
  <ConfirmDialog
    title="Delete {deleting.name}?"
    message="The rule and its hotkey are removed. This can't be undone."
    confirmLabel="Delete"
    danger
    {busy}
    onconfirm={confirmDelete}
    oncancel={() => (deleting = null)}
  />
{/if}

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .flow {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--row-h);
    padding: var(--s-1);
    border-radius: var(--r-control);
  }
  .flow:hover {
    background: var(--surface-2);
  }
  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    line-height: 1.3;
  }
  .name,
  .sum {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name.off {
    color: var(--text-muted);
  }
  .sum,
  .fired,
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .notes {
    display: flex;
    flex-wrap: wrap;
    gap: 0 var(--s-2);
    font-size: var(--fs-xs);
  }
  .running {
    color: var(--accent-text);
  }
  .failed {
    color: var(--danger);
  }
  .fired {
    font-variant-numeric: tabular-nums;
    flex: none;
  }
</style>
