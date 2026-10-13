<!--
  Preferences > MIDI & OSC, the body (a lazy chunk, loaded by MidiSection when it first scrolls
  near the viewport or the page opens on #midi): Devices (this computer) and Mappings (saved with
  the project). OSC is under Server. On macOS (no MIDI yet) only a notice: no ports, no watching.
  No props.
-->
<script lang="ts">
  import { isMac } from "$lib/platform";
  import { project } from "$lib/stores/project.svelte";
  import NotOnMac from "../NotOnMac.svelte";
  import Devices from "./Devices.svelte";
  import Mappings from "./Mappings.svelte";
</script>

{#if isMac()}
  <NotOnMac feature="MIDI" />
{:else}
<div class="midi">
  <section class="part" aria-labelledby="midi-devices-h">
    <h3 class="section-label" id="midi-devices-h">Devices <span class="note">This computer</span></h3>
    <Devices />
  </section>
  <section class="part" aria-labelledby="midi-map-h">
    <h3 class="section-label" id="midi-map-h">Mappings <span class="note">Saved with the project</span></h3>
    {#if project.current}
      <Mappings />
    {:else}
      <p class="note">Open a project to map controls.</p>
    {/if}
  </section>
</div>
{/if}

<style>
  .midi {
    display: flex;
    flex-direction: column;
    gap: var(--s-5);
    padding-top: var(--s-3);
  }
  .part {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  h3 {
    display: flex;
    align-items: baseline;
    gap: var(--s-2);
    margin: 0;
  }
  .note {
    margin: 0;
    color: var(--text-muted);
    font-family: var(--font-body);
    font-size: var(--fs-xs);
    font-weight: 400;
    letter-spacing: 0;
    text-transform: none;
  }
</style>
