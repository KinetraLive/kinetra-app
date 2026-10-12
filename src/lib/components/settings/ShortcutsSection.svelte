<!--
  Preferences > Keyboard shortcuts (`#shortcuts`; the conflict toast and the palette land here).
  The body (./shortcuts/Shortcuts.svelte: every command's keys, record / remove / reset, conflicts)
  is a lazy chunk, loaded when the section first scrolls near the viewport. No props.
-->
<script lang="ts">
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { firstVisible } from "./lazy.svelte";

  type Body = typeof import("./shortcuts/Shortcuts.svelte").default;
  let Body = $state.raw<Body | null>(null);

  function load() {
    import("./shortcuts/Shortcuts.svelte").then(
      (m) => (Body = m.default),
      () => {},
    );
  }
</script>

<SettingsSection title="Keyboard shortcuts">
  <div class:wait={!Body} {@attach firstVisible(load)}>
    {#if Body}<Body />{/if}
  </div>
</SettingsSection>

<style>
  .wait {
    min-height: calc(var(--row-h) * 4);
  }
</style>
