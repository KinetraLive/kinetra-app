<script lang="ts">
  import MessageSquare from "lucide-svelte/icons/message-square";
  import EmptyState from "$lib/components/EmptyState.svelte";
</script>

<svelte:head><title>Chat · Kinetra</title></svelte:head>

<EmptyState
  title="Chat"
  description="Show Twitch and YouTube chat on screen."
>
  {#snippet icon()}<MessageSquare size={28} />{/snippet}
</EmptyState>
