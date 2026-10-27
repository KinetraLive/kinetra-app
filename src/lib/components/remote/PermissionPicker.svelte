<!--
  PermissionPicker: what a collaborator may do. Three presets (Full operator, Data editor, Watch
  only) fill the ten checkboxes; changing a box shows "Custom". Editing graphics on air brings
  editing them off air (and unticking that one drops the other), as the server stores them.
  Part of the invite dialog's lazy chunk.
  Props: value (codes, bindable), disabled, label (the group's accessible name).
-->
<script lang="ts">
  import { PERMISSIONS, PRESETS, normalize, presetOf, type Permission } from "./permissions";

  let {
    value = $bindable([]),
    disabled = false,
    label = "Permissions",
  }: { value: string[]; disabled?: boolean; label?: string } = $props();

  const id = $props.id();
  const preset = $derived(presetOf(value));
  const has = (c: Permission) => value.includes(c);

  function toggle(c: Permission, on: boolean) {
    let next = on ? [...value, c] : value.filter((x) => x !== c);
    if (!on && c === "edit_fields") next = next.filter((x) => x !== "edit_live_fields");
    value = normalize(next);
  }
</script>

<fieldset class="picker" {disabled} aria-label={label}>
  <div class="presets" role="radiogroup" aria-label="Preset">
    {#each PRESETS as p (p.id)}
      <label class="preset vf vf-within" class:on={preset === p.id}>
        <input type="radio" name="{id}-preset" checked={preset === p.id} onchange={() => (value = normalize(p.codes))} />
        <span>{p.label}</span>
      </label>
    {/each}
    <span class="custom" class:on={preset === null} aria-hidden={preset !== null}>Custom</span>
  </div>
  <div class="boxes">
    {#each PERMISSIONS as p (p.code)}
      <label class="check">
        <input type="checkbox" checked={has(p.code)} onchange={(e) => toggle(p.code, e.currentTarget.checked)} />
        <span>{p.label}</span>
      </label>
    {/each}
  </div>
</fieldset>

<style>
  .picker {
    display: grid;
    gap: var(--s-3);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1);
  }
  .preset {
    position: relative;
    display: inline-flex;
    align-items: center;
    height: var(--control-h-sm);
    padding: 0 var(--s-3);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .preset input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .preset.on {
    background: var(--selected-bg);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .custom {
    padding: 0 var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
    visibility: hidden;
  }
  .custom.on {
    visibility: visible;
  }
  .boxes {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--s-1) var(--s-4);
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: 28px;
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .check input {
    accent-color: var(--accent);
  }
  fieldset:disabled .check,
  fieldset:disabled .preset {
    cursor: default;
    opacity: 0.6;
  }
</style>
