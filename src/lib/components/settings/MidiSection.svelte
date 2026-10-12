<!--
  Preferences > MIDI & OSC (`#midi`; the MIDI conflict toast and the palette land here). The body
  (./midi/MidiSettings.svelte: devices with status, on/off, LED feedback and Test lights; this
  project's mappings with Learn, Lights, Remove and conflicts) is a lazy chunk, loaded when the
  section first scrolls near the viewport. No props.
-->
<script lang="ts">
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { firstVisible } from "./lazy.svelte";

  type Body = typeof import("./midi/MidiSettings.svelte").default;
  let Body = $state.raw<Body | null>(null);

  function load() {
    import("./midi/MidiSettings.svelte").then(
      (m) => (Body = m.default),
      () => {},
    );
  }
</script>

<SettingsSection title="MIDI & OSC">
  <div class:wait={!Body} {@attach firstVisible(load)}>
    {#if Body}<Body />{/if}
  </div>
</SettingsSection>

<style>
  .wait {
    min-height: calc(var(--row-h) * 4);
  }
</style>
