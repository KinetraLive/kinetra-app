<!--
  On/off switch: <button role="switch" aria-checked>. Space/Enter toggle (native button).

  Props:
    checked?: boolean   bindable: <Switch bind:checked={x} label="Enable backups" />
    label: string       visible label to the left, or the aria-label when `hideLabel`
    hideLabel?: boolean use when the label is shown elsewhere (e.g. SettingsRow with `for={id}`)
    id?: string         id of the switch button (for an external <label for>)
    disabled?: boolean
    onchange?: (checked: boolean) => void   called after a user toggle
-->
<script lang="ts">
  type Props = {
    checked?: boolean;
    label: string;
    hideLabel?: boolean;
    id?: string;
    disabled?: boolean;
    onchange?: (checked: boolean) => void;
  };

  let {
    checked = $bindable(false),
    label,
    hideLabel = false,
    id,
    disabled = false,
    onchange,
  }: Props = $props();

  const autoId = $props.id();
  const btnId = $derived(id ?? autoId);

  function toggle() {
    checked = !checked;
    onchange?.(checked);
  }
</script>

{#snippet control()}
  <button
    id={btnId}
    type="button"
    role="switch"
    class="switch vf"
    aria-checked={checked}
    aria-label={hideLabel ? label : undefined}
    {disabled}
    onclick={toggle}
  >
    <span class="thumb" aria-hidden="true"></span>
  </button>
{/snippet}

{#if hideLabel}
  {@render control()}
{:else}
  <span class="row" class:disabled>
    <label for={btnId}>{label}</label>
    {@render control()}
  </span>
{/if}

<style>
  .row {
    display: inline-flex;
    align-items: center;
    gap: var(--s-3);
  }
  .row.disabled {
    opacity: 0.5;
  }
  /* 32x18 track; keyboard focus draws the viewfinder brackets around it. */
  .switch {
    --vf-radius: var(--r-full);
    position: relative;
    flex: none;
    width: 32px;
    height: 18px;
    border-radius: var(--r-full);
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .switch[aria-checked="true"] {
    background: var(--accent);
    box-shadow: none;
  }
  .switch:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .row.disabled .switch:disabled {
    opacity: 1;
  }
  .thumb {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 12px;
    height: 12px;
    border-radius: var(--r-full);
    background: var(--text);
    transition: transform var(--dur) var(--ease);
  }
  .switch[aria-checked="true"] .thumb {
    background: var(--accent-ink);
    transform: translateX(14px);
  }
</style>
