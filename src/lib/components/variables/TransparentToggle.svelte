<!--
  TransparentToggle: the "Transparent" toggle beside a colour input (VariableValuesForm, the brand
  kit). Pressed = the value is `transparent`; the parent decides what turning it off sets.

  Props:
    pressed: boolean
    label: string        the colour's name, for the accessible name ("Transparent <label>")
    ontoggle: () => void
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";

  let { pressed, label, ontoggle }: { pressed: boolean; label: string; ontoggle: () => void } = $props();
</script>

<Button size="sm" variant="ghost" class="transparent-toggle" aria-pressed={pressed} aria-label="Transparent {label}" onclick={ontoggle}>
  Transparent
</Button>

<style>
  :global(.transparent-toggle) {
    flex: none;
  }
  :global(.transparent-toggle[aria-pressed="true"]) {
    box-shadow: inset 0 0 0 1px var(--accent);
    color: var(--accent-text);
  }
</style>
