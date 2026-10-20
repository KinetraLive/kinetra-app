<!-- `$cloud-picker` in builds without cloud backup (see svelte.config.js): nothing. -->
<script lang="ts">
  let { onopened: _ }: { onopened: (folder: string) => void } = $props();
</script>
