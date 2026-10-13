<!--
  Preferences > Network & API: Server (`#server`, project settings; OSC `#osc` inside it in Advanced
  mode), Remote control (`#remote`, project-only, Pro; Connection details inside it) and MIDI & OSC (`#midi`: devices on this computer, the project's mappings). In Simple mode
  the MIDI and OSC blocks give way to one "Available in Advanced mode" hint; mappings keep working.
  No props.
-->
<!-- Runes mode: without runes Svelte compiles legacy mode, whose flag module becomes a chunk the picker loads. -->
<svelte:options runes />

<script lang="ts">
  import AdvancedHint from "$lib/components/AdvancedHint.svelte";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import MidiSection from "../MidiSection.svelte";
  import RemoteSection from "../RemoteSection.svelte";
  import ServerSection from "../ServerSection.svelte";
  import Anchor from "./Anchor.svelte";
  import ProjectOnly from "./ProjectOnly.svelte";
</script>

<Anchor id="server">
  <ProjectOnly title="Server">
    <ServerSection />
  </ProjectOnly>
</Anchor>

<Anchor id="remote">
  <ProjectOnly title="Remote control">
    <RemoteSection />
  </ProjectOnly>
</Anchor>

<Anchor id="midi">
  {#if isAdvanced()}
    <MidiSection />
  {:else}
    <AdvancedHint feature="MIDI and OSC" />
  {/if}
</Anchor>
