<!--
  "Generate template" / "Tweak with AI" (a native modal <dialog>). Generate: description (with a
  counter at the backend's limit), optional type, up to 4 brand colors and 2 fonts (prefilled from
  the project's brand kit), optional size. Tweak: an instruction. Shows the remaining quota.
  Starting a job closes the dialog: the job chip shows progress and the user keeps working.
  The core gates and checks everything again; errors use the API.md messages (a too-long text is
  kept so it can be shortened).
  Props: mode ({kind:"generate"} | {kind:"tweak", templateId, name}), onclose.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Button from "$lib/components/Button.svelte";
  import { toAppError } from "$lib/api";
  import { aiGenerate, aiLimits, aiQuota, aiTweak } from "$lib/api/ai";
  import { cloudMessage } from "$lib/components/license/messages";
  import { project } from "$lib/stores/project.svelte";
  import { TEMPLATE_FONTS, type AiLimits, type AiQuota, type AiTaskQuota } from "$lib/types";
  import { jobs } from "./jobs.svelte";

  type Mode = { kind: "generate" } | { kind: "tweak"; templateId: string; name: string };
  let { mode, onclose }: { mode: Mode; onclose: () => void } = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let limits = $state.raw<AiLimits>({ maxChars: 2000, maxColors: 4, maxFonts: 2, types: [] });
  let quota = $state.raw<AiQuota | null>(null);
  let text = $state("");
  let type = $state("");
  const brand = project.current?.settings.brand ?? null;
  const isHex = (c: string | null | undefined): c is string => !!c && /^#[0-9a-f]{6}$/i.test(c);
  let colors = $state<string[]>(
    brand ? [brand.accent, brand.secondary, brand.text, brand.plate].filter(isHex).slice(0, 4) : [],
  );
  let fonts = $state<string[]>(
    brand ? [brand.fontHeading, brand.fontBody].filter((f): f is string => !!f && TEMPLATE_FONTS.some((t) => t.name === f)).slice(0, 2) : [],
  );
  let width = $state<number | null>(null);
  let height = $state<number | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);

  const count = $derived([...text].length);
  const tooLong = $derived(count > limits.maxChars);
  const generate = $derived(mode.kind === "generate");
  const q = $derived<AiTaskQuota | null>(quota ? (generate ? quota.generate_template : quota.tweak_template) : null);
  /** AI is switched off (globally or for this account): say so up front, nothing to send. */
  const off = $derived(
    !quota ? null : quota.disabled_for_account ? "AI is turned off for this account. Contact hello@kinetra.live." : !quota.enabled ? "AI is turned off right now. Everything else in Kinetra works as usual." : null,
  );
  const TYPE_NAMES: Record<string, string> = {
    lower_third: "Lower third",
    full_screen: "Full screen",
    title: "Title",
    bug: "Corner bug",
    ticker: "Ticker",
    scoreboard: "Scoreboard",
    countdown: "Countdown",
    schedule: "Schedule",
    list: "List",
    other: "Other",
  };

  onMount(() => {
    dialog?.showModal();
    aiLimits().then((l) => (limits = l), () => {});
    aiQuota().then((x) => (quota = x), () => {});
  });

  function quotaText(t: AiTaskQuota): string {
    const left = Math.max(0, t.limit - t.used);
    const period = t.period === "month" ? " this month" : t.period === "total" ? "" : ` per ${t.period}`;
    let s = `${left} of ${t.limit} left${period}`;
    if (t.day_limit != null && t.day_used != null) s += ` · ${Math.max(0, t.day_limit - t.day_used)} of ${t.day_limit} today`;
    return s;
  }

  function startError(e: unknown): string {
    const err = toAppError(e);
    if (err.apiCode === "ai_quota_exhausted") {
      const at = Number((err.details as { resets_at?: number } | undefined)?.resets_at);
      const when = at ? ` It resets ${new Date(at * 1000).toLocaleDateString("en-US", { month: "short", day: "numeric" })}.` : "";
      return `${err.message}${when}`;
    }
    return cloudMessage(e);
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (busy || tooLong || !text.trim()) return;
    busy = true;
    error = null;
    try {
      const j =
        mode.kind === "generate"
          ? await aiGenerate({
              prompt: text,
              templateType: type || null,
              brandColors: colors,
              fonts,
              width: width && height ? width : null,
              height: width && height ? height : null,
            })
          : await aiTweak(mode.templateId, text);
      jobs.start();
      jobs.put(j);
      dialog?.close();
      onclose();
    } catch (err) {
      error = startError(err);
    } finally {
      busy = false;
    }
  }
</script>

<dialog bind:this={dialog} class="ai" aria-labelledby="{id}-t" onclose={onclose}>
  <form onsubmit={submit}>
    <h2 id="{id}-t">{generate ? "Generate template" : `Tweak “${mode.kind === "tweak" ? mode.name : ""}”`}</h2>
    <label for="{id}-text">{generate ? "Describe the graphic" : "What should change?"}</label>
    <textarea
      id="{id}-text"
      rows={generate ? 5 : 3}
      bind:value={text}
      aria-describedby="{id}-count"
      placeholder={generate ? "Lower third for a football match, club colors, bold names" : "Make it blue and move the name above the title"}
    ></textarea>
    <span id="{id}-count" class="count tnum" class:over={tooLong}>{count} / {limits.maxChars}</span>

    {#if generate}
      <div class="grid">
        <label for="{id}-type">Type</label>
        <select id="{id}-type" bind:value={type}>
          <option value="">Any</option>
          {#each limits.types as t (t)}<option value={t}>{TYPE_NAMES[t] ?? t}</option>{/each}
        </select>

        <span class="lbl">Brand colors</span>
        <span class="row">
          {#each colors as _, i (i)}
            <input type="color" aria-label="Brand color {i + 1}" bind:value={colors[i]} />
            <button type="button" class="x" aria-label="Remove brand color {i + 1}" onclick={() => colors.splice(i, 1)}>×</button>
          {/each}
          {#if colors.length < limits.maxColors}
            <Button size="sm" variant="ghost" onclick={() => colors.push("#ffffff")}>Add color</Button>
          {/if}
        </span>

        <span class="lbl">Fonts</span>
        <span class="row">
          {#each [0, 1].slice(0, limits.maxFonts) as i (i)}
            <select aria-label={i === 0 ? "Heading font" : "Body font"} value={fonts[i] ?? ""} onchange={(e) => {
              const v = e.currentTarget.value;
              const next = [...fonts];
              if (v) next[i] = v; else next.splice(i, 1);
              fonts = next.filter(Boolean);
            }}>
              <option value="">{i === 0 ? "Any heading font" : "Any body font"}</option>
              {#each TEMPLATE_FONTS as f (f.slug)}<option value={f.name}>{f.name}</option>{/each}
            </select>
          {/each}
        </span>

        <span class="lbl">Size</span>
        <span class="row">
          <input class="num mono" type="number" min="16" max="8192" aria-label="Width" placeholder="Width" bind:value={width} />
          <span class="muted">×</span>
          <input class="num mono" type="number" min="16" max="8192" aria-label="Height" placeholder="Height" bind:value={height} />
        </span>
      </div>
    {/if}

    {#if off}<p class="warn" role="status">{off}</p>{:else if q}<p class="muted tnum">{quotaText(q)}</p>{/if}
    {#if error}<p class="bad" role="alert">{error}</p>{/if}

    <div class="acts">
      <Button type="submit" variant="primary" loading={busy} disabled={tooLong || !text.trim() || !!off}>
        {generate ? "Generate" : "Tweak"}
      </Button>
      <Button variant="ghost" onclick={() => { dialog?.close(); onclose(); }}>Cancel</Button>
    </div>
  </form>
</dialog>

<style>
  .ai {
    width: min(560px, calc(100vw - 32px));
    padding: var(--s-5);
    border: 0;
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text);
  }
  .ai::backdrop {
    background: color-mix(in srgb, var(--bg) 70%, transparent);
  }
  form {
    display: grid;
    gap: var(--s-2);
  }
  h2 {
    margin: 0 0 var(--s-2);
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  label,
  .lbl {
    font-weight: 600;
  }
  textarea,
  select,
  .num {
    padding: var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text);
    font: inherit;
  }
  textarea {
    resize: vertical;
  }
  textarea:focus-visible,
  select:focus-visible,
  input:focus-visible,
  .x:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }
  .count {
    justify-self: end;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .count.over {
    color: var(--danger);
  }
  .grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    align-items: center;
    gap: var(--s-2) var(--s-3);
    margin-top: var(--s-2);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1);
  }
  input[type="color"] {
    width: 32px;
    height: 24px;
    padding: 0;
    border: 0;
    background: none;
  }
  .x {
    padding: 0 var(--s-1);
    border: 0;
    border-radius: var(--r-control);
    background: none;
    color: var(--text-muted);
    cursor: pointer;
  }
  .num {
    width: 90px;
  }
  p {
    margin: 0;
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .bad {
    color: var(--danger);
  }
  .warn {
    color: var(--warning);
  }
  .acts {
    display: flex;
    gap: var(--s-2);
    margin-top: var(--s-2);
  }
</style>
