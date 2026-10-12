<!--
  TallyDot: THE status indicator. 8px round dot.
    program  red (--tally-program)  on air; pulses softly (off under reduced motion)
    preview  green (--tally-preview) preview / ready / connected
    selected accent
    warning  amber (not a tally colour: problems like "port in use")
    idle     muted
  Colour alone never carries meaning: pass `label` (announced to screen readers) or put visible
  text next to the dot. Without `label` the dot is decorative (aria-hidden).
  In light theme program/preview dots get a 1px darker ring so their edge reaches 3:1.

  Props: state, label?, pulse? (default: true for program), size? (px, default 8),
  title? (native hover hint; does not replace `label` for screen readers).
  The pulse animates opacity only and stops under prefers-reduced-motion.
-->
<script lang="ts">
  type State = "program" | "preview" | "selected" | "warning" | "idle";

  let {
    state = "idle",
    label,
    pulse,
    size = 8,
    title,
  }: { state?: State; label?: string; pulse?: boolean; size?: number; title?: string } = $props();

  const pulsing = $derived(pulse ?? state === "program");
</script>

<span
  class="tally {state}"
  class:pulse={pulsing}
  style:--size="{size}px"
  {title}
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
></span>

<style>
  .tally {
    display: inline-block;
    flex: none;
    width: var(--size);
    height: var(--size);
    border-radius: var(--r-full);
    background: var(--dot);
    box-shadow: inset 0 0 0 1px var(--ring, transparent);
    vertical-align: middle;
  }

  .program {
    --dot: var(--tally-program);
    --ring: var(--tally-program-ring);
  }
  .preview {
    --dot: var(--tally-preview);
    --ring: var(--tally-preview-ring);
  }
  .selected {
    --dot: var(--accent);
    --ring: var(--focus);
  }
  .warning {
    --dot: var(--warning);
  }
  .idle {
    --dot: var(--text-muted);
  }

  .pulse {
    animation: tally-pulse var(--tally-pulse) ease-in-out infinite;
  }

  @keyframes tally-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.45;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
</style>
