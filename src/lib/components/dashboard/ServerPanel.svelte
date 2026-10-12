<!--
  Dashboard server/LAN panel: output server status (tally dot + text), the local URL (mono, copy),
  LAN access on/off with the LAN URL (mono, copy) when on, the port (mono), and a link to
  Preferences > Server. Port and LAN come from the project settings; URLs from the live status.

  Props:
    status: ServerStatus | null   (the page watches the server store)
-->
<script lang="ts">
  import ArrowRight from "lucide-svelte/icons/arrow-right";
  import Copy from "lucide-svelte/icons/copy";
  import IconButton from "$lib/components/IconButton.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { preferencesLink } from "$lib/components/settings/open";
  import { project } from "$lib/stores/project.svelte";
  import { copyUrl, hostPort } from "$lib/stores/server.svelte";
  import type { ServerStatus } from "$lib/types";

  let { status }: { status: ServerStatus | null } = $props();
  const uid = $props.id();

  const settings = $derived(project.current?.settings);
  const port = $derived(
    status && status.state !== "stopped" ? status.port : (settings?.port ?? 7770),
  );
  const lanOn = $derived(settings?.lanAccess ?? false);
  const url = $derived(status?.state === "running" ? status.url : null);
  const lanUrl = $derived(status?.state === "running" ? status.lanUrl : null);

  const state = $derived.by(() => {
    switch (status?.state) {
      case "running":
        return { dot: "preview", text: "Running" } as const;
      case "starting":
        return { dot: "idle", text: "Starting…" } as const;
      case "error":
        return {
          dot: "warning",
          text: status.code === "portInUse" ? "Port in use" : "Couldn't start",
        } as const;
      default:
        return { dot: "idle", text: "Stopped" } as const;
    }
  });
</script>

<section class="panel" aria-labelledby="{uid}-h">
  <h2 id="{uid}-h" class="section-label">Output server</h2>
  <dl class="list">
    <div class="item">
      <dt>Status</dt>
      <dd>
        <span class="status" class:warn={status?.state === "error"} role="status">
          <TallyDot state={state.dot} />{state.text}
        </span>
      </dd>
    </div>
    <div class="item">
      <dt>Local</dt>
      <dd>
        {#if url}
          <span class="mono value" title={url}>{hostPort(url)}</span>
          <IconButton label="Copy local URL" size="sm" tooltip="left" onclick={() => copyUrl(url)}>
            <Copy size={14} />
          </IconButton>
        {:else}
          <span class="muted">—</span>
        {/if}
      </dd>
    </div>
    <div class="item">
      <dt>LAN</dt>
      <dd>
        {#if lanOn && lanUrl}
          <span class="mono value" title={lanUrl}>{hostPort(lanUrl)}</span>
          <IconButton label="Copy LAN URL" size="sm" tooltip="left" onclick={() => copyUrl(lanUrl)}>
            <Copy size={14} />
          </IconButton>
        {:else}
          <span class="muted">{lanOn ? "On" : "Off"}</span>
        {/if}
      </dd>
    </div>
    <div class="item">
      <dt>Port</dt>
      <dd><span class="mono value">{port}</span></dd>
    </div>
  </dl>
  <a class="more vf" href="/settings#server" onclick={preferencesLink("server")}>
    Server preferences <ArrowRight size={14} aria-hidden="true" />
  </a>
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .list {
    margin: 0;
  }
  .item {
    display: grid;
    grid-template-columns: 5rem minmax(0, 1fr);
    align-items: center;
    gap: var(--s-3);
    min-height: var(--row-h);
    border-top: 1px solid var(--line);
  }
  .item:last-child {
    border-bottom: 1px solid var(--line);
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
    margin: 0;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
  }
  .warn {
    color: var(--warning);
  }
  .value {
    min-width: 0;
    overflow: hidden;
    font-size: var(--fs-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
    user-select: text;
  }
  .muted {
    color: var(--text-muted);
  }
  .more {
    --vf-radius: var(--r-control);
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    align-self: flex-start;
    height: var(--control-h-sm);
    margin-top: var(--s-1);
    border-radius: var(--r-control);
    font-size: var(--fs-sm);
    font-weight: 500;
    text-decoration: none;
  }
  .more:hover {
    text-decoration: underline;
  }
</style>
