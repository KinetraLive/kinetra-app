<!--
  One settings row, two columns: label (13px) + muted description on the left, the control on the
  right (left-aligned in its column, so every control in a section lines up). A full-width 1px
  divider sits above each row. Use inside SettingsSection, which sets the columns
  (--settings-cols) and is the size container: below 560px the row stacks.

  Props:
    label: string
    description?: string
    for?: string         id of the control; makes the label a real <label for>
    stacked?: boolean    the control spans the full width below the label (lists, text areas)
    children: Snippet    the control(s)
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    label: string;
    description?: string;
    for?: string;
    stacked?: boolean;
    children: Snippet;
  };

  let { label, description, for: forId, stacked = false, children }: Props = $props();
</script>

<div class="settings-row" class:stacked>
  <div class="text">
    {#if forId}
      <label class="label" for={forId}>{label}</label>
    {:else}
      <span class="label">{label}</span>
    {/if}
    {#if description}<p class="desc">{description}</p>{/if}
  </div>
  <div class="control">{@render children()}</div>
</div>

<style>
  .settings-row {
    display: grid;
    grid-template-columns: var(--settings-cols, minmax(220px, 5fr) minmax(0, 7fr));
    align-items: center;
    gap: var(--s-2) var(--s-6);
    min-height: calc(var(--row-h) + var(--s-4));
    padding: var(--s-2) 0;
    border-top: 1px solid var(--line);
  }
  .settings-row.stacked {
    grid-template-columns: minmax(0, 1fr);
    align-items: stretch;
  }
  .text {
    min-width: 0;
  }
  .label {
    display: block;
    font-size: var(--fs-md);
    font-weight: 500;
    line-height: 1.35;
    color: var(--text);
  }
  .desc {
    margin-top: 2px;
    max-width: 60ch;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    line-height: 1.4;
  }
  .control {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }

  @container (max-width: 560px) {
    .settings-row {
      grid-template-columns: minmax(0, 1fr);
      align-items: start;
    }
  }
</style>
