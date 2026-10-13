<!--
  Preferences > Network & API: Server (`#server`, project settings; OSC `#osc` inside it in Advanced
  mode), Remote control (`#remote`, project-only, Pro, only in builds with remote control: `$remote`;
  Connection details inside it) and MIDI & OSC (`#midi`: devices on this computer, the project's mappings). In Simple mode
  the MIDI and OSC blocks give way to one "Available in Advanced mode" hint; mappings keep working.
  No props.
-->
<!-- Runes mode: without runes Svelte compiles legacy mode, whose flag module becomes a chunk the picker loads. -->
<svelte:options runes />

<script lang="ts">
  import AdvancedHint from "$lib/components/AdvancedHint.svelte";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import MidiSection from "../MidiSection.svelte";
  import { loadSection } from "$remote";
  import ServerSection from "../ServerSection.svelte";
  import Anchor from "./Anchor.svelte";
  import ProjectOnly from "./ProjectOnly.svelte";
</script>

<Anchor id="server">
  <ProjectOnly title="Server">
    <ServerSection />
  </ProjectOnly>
</Anchor>

{#if loadSection}
  <Anchor id="remote">
    <ProjectOnly title="Remote control">
      {#await loadSection() then RemoteSection}<RemoteSection />{/await}
    </ProjectOnly>
  </Anchor>
{/if}

<Anchor id="midi">
  {#if isAdvanced()}
    <MidiSection />
  {:else}
    <AdvancedHint feature="MIDI and OSC" />
  {/if}
</Anchor>
