<!--
  Preferences > Style & brand (project settings): the project's style and its brand kit. The rows
  (./StyleBrand.svelte, with the styles list, color, font and logo pickers) are a lazy chunk,
  loaded when the section first scrolls near the viewport. No props.
-->
<script lang="ts">
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { firstVisible } from "./lazy.svelte";

  type Body = typeof import("./StyleBrand.svelte").default;
  let Body = $state.raw<Body | null>(null);

  function load() {
    import("./StyleBrand.svelte").then(
      (m) => (Body = m.default),
      () => {},
    );
  }
</script>

<SettingsSection title="Style & brand">
  <div class:wait={!Body} {@attach firstVisible(load)}>
    {#if Body}<Body />{/if}
  </div>
</SettingsSection>

<style>
  .wait {
    min-height: calc(var(--row-h) + var(--s-4));
  }
</style>
