<!--
  Small compact label (20px). Status badges lead with a TallyDot.

  Props:
    tone?: "neutral" | "accent" | "success" | "warning" | "danger" | "onAir"   (default "neutral")
    tally?: "program" | "preview" | "selected" | "warning" | "idle"
                        renders a TallyDot of that state before the text (wins over `dot`)
    dot?: boolean       leading TallyDot derived from the tone:
                        onAir -> program, warning -> warning, accent -> selected, others -> idle.
                        For "connected / ready" pass tally="preview" explicitly.
    children: Snippet
    ...rest             passed to <span>

  Tally red appears only for `onAir`. `danger` is for destructive/error states (--danger red, never
  tally red).
  `success` reads as the accent (there is no success green).
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import TallyDot from "./TallyDot.svelte";

  type Tone = "neutral" | "accent" | "success" | "warning" | "danger" | "onAir";
  type Tally = "program" | "preview" | "selected" | "warning" | "idle";

  type Props = HTMLAttributes<HTMLSpanElement> & {
    tone?: Tone;
    tally?: Tally;
    dot?: boolean;
    children: Snippet;
  };

  let {
    tone = "neutral",
    tally,
    dot = false,
    children,
    class: className = "",
    ...rest
  }: Props = $props();

  const TONE_TALLY: Record<Tone, Tally> = {
    onAir: "program",
    warning: "warning",
    accent: "selected",
    neutral: "idle",
    success: "idle",
    danger: "idle",
  };

  const dotState = $derived<Tally | undefined>(tally ?? (dot ? TONE_TALLY[tone] : undefined));
</script>

<span {...rest} class="badge {tone} {className}">
  {#if dotState}<TallyDot state={dotState} />{/if}
  {@render children()}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: 20px;
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text-muted);
    font-family: var(--font-body);
    font-size: var(--fs-xs);
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
  }
  /* Accent text is only AA on surface-2 and below in the light theme. */
  .accent,
  .success {
    background: var(--surface-2);
    color: var(--accent-text);
  }
  .warning {
    color: var(--warning);
  }
  .danger {
    color: var(--danger);
  }
  .onAir {
    box-shadow: inset 0 0 0 1px var(--tally-program);
    color: var(--text);
    font-family: var(--font-heading);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: var(--tracking-label);
  }
</style>
