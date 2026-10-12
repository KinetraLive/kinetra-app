<!--
  Preferences > Advanced: Project location (`#location`), Backups (`#backups`), Settings file
  (`#settings-file`) and Reset (`#reset`). Location and Backups need an open project; Settings file
  and Reset also work without one. No props.
-->
<!-- Runes mode: without runes Svelte compiles legacy mode, whose flag module becomes a chunk the picker loads. -->
<svelte:options runes />

<script lang="ts">
  import BackupsSection from "../BackupsSection.svelte";
  import LocationSection from "../LocationSection.svelte";
  import ResetSection from "../ResetSection.svelte";
  import SettingsFileSection from "../SettingsFileSection.svelte";
  import Anchor from "./Anchor.svelte";
  import ProjectOnly from "./ProjectOnly.svelte";
</script>

<Anchor id="location">
  <ProjectOnly title="Project location">
    <LocationSection />
  </ProjectOnly>
</Anchor>
<Anchor id="backups">
  <ProjectOnly title="Backups">
    <BackupsSection />
  </ProjectOnly>
</Anchor>
<Anchor id="settings-file">
  <SettingsFileSection />
</Anchor>
<Anchor id="reset">
  <ResetSection />
</Anchor>
