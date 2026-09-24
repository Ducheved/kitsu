<script lang="ts">
  import type { Attention, Status } from "../lib/types";

  let { attention, status, size = 14 }: { attention: Attention; status?: Status; size?: number } = $props();

  const failing = $derived(status?.kind === "failed" || status?.kind === "interrupted" || (status?.kind === "review" && status.verdict === "failing"));
  const label = $derived(
    { needs_you: "needs you", working: "working", ready: "ready", waiting: "waiting", quiet: "done" }[attention],
  );
</script>

<svg class="glyph {attention}" class:failing width={size} height={size} viewBox="0 0 16 16" role="img" aria-label={label}>
  {#if attention === "needs_you"}
    <circle cx="8" cy="8" r="5" />
  {:else if attention === "working"}
    <circle class="track" cx="8" cy="8" r="5.5" />
    <path class="arc" d="M8 2.5 A5.5 5.5 0 0 1 13.5 8" />
  {:else if attention === "ready"}
    <circle class="ring" cx="8" cy="8" r="5" />
  {:else if attention === "waiting"}
    <circle class="dash" cx="8" cy="8" r="5" />
  {:else}
    <path class="check" d="M4.5 8.3 L7 10.6 L11.5 5.6" />
  {/if}
</svg>

<style>
  .glyph {
    flex: none;
  }
  .needs_you circle {
    fill: var(--warn);
  }
  .failing.needs_you circle {
    fill: var(--bad);
  }
  .track {
    fill: none;
    stroke: var(--work-soft);
    stroke-width: 2;
  }
  .arc {
    fill: none;
    stroke: var(--work);
    stroke-width: 2;
    stroke-linecap: round;
    transform-origin: 8px 8px;
    animation: spin 1.1s linear infinite;
  }
  .ring {
    fill: none;
    stroke: var(--muted);
    stroke-width: 1.6;
  }
  .dash {
    fill: none;
    stroke: var(--faint);
    stroke-width: 1.4;
    stroke-dasharray: 2.2 2.2;
  }
  .check {
    fill: none;
    stroke: var(--faint);
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
