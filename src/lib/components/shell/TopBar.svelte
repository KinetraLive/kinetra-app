<!--
  App top bar, left to right: Kinetra logo (mark centred over the rail) · divider · project
  switcher · flexible space · cloud backup's chip (while the open project backs up: a lazy chunk;
  `cloud`) · remote control's chip (only in builds with remote control and while
  it is on: a lazy chunk with its panel; `$remote`) · output server status · theme toggle. Watches remote control (`remote`: the view and
  the cue list's position, which Control's cue strip reads) on every app page.

  Server status is a tally dot + text, never the program (on-air) colour:
    running      preview dot + mono URL (127.0.0.1:7770, or LAN 192.168.x.x:7770) + copy button
    starting     idle dot + "Starting…"
    port in use  warning dot + "Port 7770 is in use", linking to Preferences > Server
    other error  warning dot + "Server couldn't start", the backend message in a tooltip
    stopped      nothing
  Watches the server status store while mounted. No props.
-->
<script lang="ts">
  import Copy from "lucide-svelte/icons/copy";
  import IconButton from "$lib/components/IconButton.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import ThemeToggle from "$lib/components/ThemeToggle.svelte";
  import Tooltip from "$lib/components/Tooltip.svelte";
  import { copyUrl, hostPort, server } from "$lib/stores/server.svelte";
  import { preferencesLink } from "$lib/components/settings/open";
  import Logo from "./Logo.svelte";
  import ProjectSwitcher from "./ProjectSwitcher.svelte";
  import { remote } from "$lib/components/remote/state.svelte";
  import { loadChip } from "$remote";
  import { cloud } from "$lib/components/cloud/state.svelte";
  import { history, loadHistory } from "$lib/components/cloud/history/open.svelte";

  $effect(() => server.watch());
  $effect(() => remote.watch());
  $effect(() => cloud.watch());
  /** Cloud backup's chip: a lazy chunk, only while the open project backs up. */
  const loadCloudChip = () => import("$lib/components/cloud/CloudChip.svelte").then((m) => m.default);

  const status = $derived(server.status);
  /** What "Copy output URL" copies: the LAN URL when there is one (it works everywhere). */
  const copyTarget = $derived(
    status?.state === "running" ? (status.lanUrl ?? status.url) : null,
  );
</script>

<header class="topbar">
  <Logo />
  <span class="divider" aria-hidden="true"></span>
  <ProjectSwitcher />
  <div class="right">
    {#if loadChip && remote.view?.on}
      {#await loadChip() then Chip}<Chip />{/await}
    {/if}
    {#if cloud.view?.access === "available" && cloud.view.project?.backup}
      {#await loadCloudChip() then CloudChip}<CloudChip />{/await}
    {/if}
    <div class="status" role="status">
      {#if status?.state === "running"}
        <span class="state">
          <TallyDot state="preview" />
          {#if status.lanUrl}
            <span class="mono url" title="Also at {hostPort(status.url)}"
              >LAN {hostPort(status.lanUrl)}</span
            >
          {:else}
            <span class="mono url">{hostPort(status.url)}</span>
          {/if}
        </span>
        <IconButton
          label="Copy output URL"
          size="sm"
          onclick={() => copyTarget && copyUrl(copyTarget)}
        >
          <Copy size={14} />
        </IconButton>
      {:else if status?.state === "starting"}
        <span class="state muted"><TallyDot state="idle" />Starting…</span>
      {:else if status?.state === "error" && status.code === "portInUse"}
        <a class="state link vf" href="/settings#server" onclick={preferencesLink("server")}>
          <TallyDot state="warning" />Port {status.port} is in use
        </a>
      {:else if status?.state === "error"}
        <Tooltip text={status.message} placement="bottom" focusable>
          <span class="state"><TallyDot state="warning" />Server couldn't start</span>
        </Tooltip>
      {/if}
    </div>
    <ThemeToggle />
  </div>
</header>
{#if history.open}
  {#await loadHistory() then History}<History open={history.open} onclose={history.close} />{/await}
{/if}

<style>
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    /* Centres the logo's mark over the --rail-w rail: in the 32px-high artwork the mark's centre
       is 16px from its left edge (inside the logo's --s-1 padding). */
    padding: 0 var(--s-2) 0 calc(var(--rail-w) / 2 - 16px - var(--s-1));
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .divider {
    flex: none;
    width: 1px;
    height: 20px;
    margin: 0 var(--s-1);
    background: var(--line);
  }
  .right {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--s-2);
    margin-left: auto;
  }
  /* Fixed height in every state, so the bar never shifts; the theme toggle stays put because the
     group is right-aligned. */
  .status {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    height: var(--control-h);
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    height: var(--control-h-sm);
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    color: var(--text);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .muted {
    color: var(--text-muted);
  }
  .url {
    font-size: var(--fs-sm);
  }
  .link {
    --vf-radius: var(--r-control);
    text-decoration: none;
  }
  .link:hover {
    text-decoration: underline;
  }
</style>
