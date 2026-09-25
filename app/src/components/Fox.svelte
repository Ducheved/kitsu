<script lang="ts" module>
  export type FoxState = "idle" | "working" | "happy" | "asking" | "sleeping" | "perk";
</script>

<script lang="ts">
  // Kitsu's fox. Decoration only: whatever state it shows is also said in
  // words next to it, so it's hidden from assistive tech. With reduced
  // motion every state still reads from its still frame (eyes, tilt, ears, z's).
  import { onDestroy } from "svelte";

  let { state: mood = "idle", size = 40 }: { state?: FoxState; size?: number } = $props();

  // Hovering the fox makes it glance toward the side the pointer came from.
  let look = $state<"" | "left" | "right">("");
  let lookTimer: ReturnType<typeof setTimeout> | undefined;
  function glance(e: PointerEvent) {
    if (mood === "sleeping") return;
    const r = (e.currentTarget as Element).getBoundingClientRect();
    look = e.clientX < r.left + r.width / 2 ? "left" : "right";
    clearTimeout(lookTimer);
    lookTimer = setTimeout(() => (look = ""), 1100);
  }
  onDestroy(() => clearTimeout(lookTimer));
</script>

<svg class="fox {mood}" class:look-left={look === "left"} class:look-right={look === "right"} width={size} height={size} viewBox="0 0 48 48" aria-hidden="true" onpointerenter={glance}>
  <g class="all">
    <g class="tail">
      <path class="fur" d="M29 44.5 C39 46 45.5 40 43.5 31.5 C42.6 27.8 38 27.4 37.6 31.2 C37.2 35.6 34.5 38.8 28.5 39.2 Z" />
      <path class="face" d="M43.5 31.5 C42.6 27.8 38 27.4 37.6 31.2 C39.4 32.4 41.6 32.5 43.5 31.5 Z" />
    </g>
    <g class="body">
      <path class="fur" d="M18 25 L13.5 44.5 L30.5 44.5 L26 25 Z" />
      <path class="face" d="M18.6 31 L22 41.5 L25.4 31 Z" />
    </g>
    <g class="head">
      <g class="look">
        <g class="ear left">
          <path class="fur" d="M11.8 16 L13 3.8 L20.5 12 Z" />
          <path class="inner" d="M14 12.2 L14.6 7.4 L17.6 10.8" />
        </g>
        <g class="ear right">
          <path class="fur" d="M32.2 16 L31 3.8 L23.5 12 Z" />
          <path class="inner" d="M30 12.2 L29.4 7.4 L26.4 10.8" />
        </g>
        <path class="fur" d="M9 21.5 L12 13.5 L17 11.6 L22 13.4 L27 11.6 L32 13.5 L35 21.5 L22 31.5 Z" />
        <path class="face" d="M9 21.5 L15.5 22.6 L22 26 L28.5 22.6 L35 21.5 L22 31.5 Z" />
        <g class="gaze">
          <g class="eyes">
            {#if mood === "happy"}
              <path class="lid" d="M15.6 19.8 Q17.4 17.6 19.2 19.8" />
              <path class="lid" d="M24.8 19.8 Q26.6 17.6 28.4 19.8" />
            {:else if mood === "sleeping"}
              <path class="lid" d="M15.6 19 Q17.4 20.8 19.2 19" />
              <path class="lid" d="M24.8 19 Q26.6 20.8 28.4 19" />
            {:else}
              <circle class="ink" cx="17.4" cy="19.2" r="1.45" />
              <circle class="ink" cx="26.6" cy="19.2" r="1.45" />
            {/if}
          </g>
          <path class="ink" d="M20.5 29.2 L23.5 29.2 L22 31.2 Z" />
        </g>
      </g>
    </g>
    {#if mood === "sleeping"}
      <g class="zz">
        <text x="36" y="12">z</text>
        <text x="40.5" y="7" class="small">z</text>
      </g>
    {/if}
  </g>
</svg>

<style>
  .fox {
    flex: none;
    overflow: visible;
    display: block;
  }
  .fox * {
    transform-box: view-box;
  }
  .fur {
    fill: var(--accent);
    stroke: var(--accent);
    stroke-width: 1.2;
    stroke-linejoin: round;
  }
  /* Outlined in the fur color so the white mask still has an edge on a white card. */
  .face {
    fill: var(--fox-face);
    stroke: var(--accent);
    stroke-width: 1;
    stroke-linejoin: round;
  }
  .inner {
    fill: none;
    stroke: var(--fox-face);
    stroke-width: 1.1;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0.7;
  }
  .ink {
    fill: var(--fox-ink);
  }
  .lid {
    fill: none;
    stroke: var(--fox-ink);
    stroke-width: 1.5;
    stroke-linecap: round;
  }
  .zz text {
    fill: var(--faint);
    font: 600 8px var(--font);
  }
  .zz .small {
    font-size: 6px;
  }

  .tail {
    transform-origin: 29px 42px;
  }
  .head {
    transform-origin: 22px 30px;
  }
  .ear.left {
    transform-origin: 16px 14px;
  }
  .ear.right {
    transform-origin: 28px 14px;
  }
  .eyes {
    transform-origin: 22px 19.2px;
  }
  .all {
    transform-origin: 22px 45px;
  }

  /* idle: a slow breath, a blink every few seconds, an ear now and then. */
  .idle .all {
    animation: breathe 4.8s ease-in-out infinite;
  }
  .idle .eyes {
    animation: blink 5s infinite;
  }
  .idle .ear.right {
    animation: twitch 7s 1.5s infinite;
  }

  /* working: tail keeps time, head bobs on the half beat. */
  .working .tail {
    animation: sway 0.8s ease-in-out infinite alternate;
  }
  .working .head {
    animation: bob 0.4s ease-in-out infinite alternate;
  }
  .working .eyes {
    animation: blink 4s 0.6s infinite;
  }

  /* happy: one hop, then the tail waves twice. The still frame is the smile. */
  .happy .all {
    animation: hop 0.6s var(--ease) 0.1s;
  }
  .happy .tail {
    animation: wag 0.28s ease-in-out 0.2s 4 alternate;
  }

  /* perk: something just finished. Ears up (that's the still frame), one small hop. */
  .perk .ear {
    transform: translateY(-1.4px) scaleY(1.08);
  }
  .perk .all {
    animation: perk 0.6s var(--ease) 0.15s;
  }
  .perk .eyes {
    animation: blink 5s 1.4s infinite;
  }

  /* asking: the head is tilted even when nothing moves. */
  .asking .head {
    transform: rotate(-9deg);
    animation: wonder 3.2s ease-in-out infinite;
  }
  .asking .ear.left {
    animation: twitch 3.2s 0.8s infinite;
  }

  /* sleeping: head down, slow breath, z's drifting up. */
  .sleeping .head {
    transform: translateY(1.5px) rotate(5deg);
  }
  .sleeping .all {
    animation: breathe-deep 4s ease-in-out infinite;
  }
  .sleeping .zz text {
    animation: drift 3.6s ease-in-out infinite;
  }
  .sleeping .zz .small {
    animation-delay: 1.2s;
  }

  /* Hover: the head turns a little toward the pointer, eyes follow, and
     that side's ear flicks once. */
  .look,
  .gaze {
    transition: transform var(--quick) var(--ease);
  }
  .look {
    transform-origin: 22px 30px;
  }
  .look-left .look {
    transform: rotate(-6deg);
  }
  .look-right .look {
    transform: rotate(6deg);
  }
  .look-left .gaze {
    transform: translateX(-1px);
  }
  .look-right .gaze {
    transform: translateX(1px);
  }
  .look-left .ear.left,
  .look-right .ear.right {
    animation: flick 0.45s var(--ease);
  }

  @keyframes blink {
    0%,
    92%,
    100% {
      transform: scaleY(1);
    }
    95% {
      transform: scaleY(0.1);
    }
  }
  @keyframes twitch {
    0%,
    88%,
    100% {
      transform: rotate(0);
    }
    91% {
      transform: rotate(-9deg);
    }
    94% {
      transform: rotate(3deg);
    }
  }
  @keyframes sway {
    from {
      transform: rotate(-7deg);
    }
    to {
      transform: rotate(8deg);
    }
  }
  @keyframes bob {
    to {
      transform: translateY(0.9px);
    }
  }
  @keyframes hop {
    0%,
    100% {
      transform: none;
    }
    20% {
      transform: scale(1.04, 0.94);
    }
    50% {
      transform: translateY(-6px) scale(0.98, 1.03);
    }
    80% {
      transform: scale(1.03, 0.96);
    }
  }
  @keyframes wag {
    to {
      transform: rotate(-12deg);
    }
  }
  @keyframes wonder {
    0%,
    100% {
      transform: rotate(-9deg);
    }
    50% {
      transform: rotate(-13deg) translateY(-0.4px);
    }
  }
  @keyframes breathe {
    0%,
    100% {
      transform: none;
    }
    50% {
      transform: scale(1.012, 1.018);
    }
  }
  @keyframes breathe-deep {
    0%,
    100% {
      transform: none;
    }
    50% {
      transform: scale(1.025, 0.975);
    }
  }
  @keyframes perk {
    0%,
    100% {
      transform: none;
    }
    35% {
      transform: translateY(-4px) scale(0.99, 1.03);
    }
    65% {
      transform: scale(1.02, 0.97);
    }
  }
  @keyframes flick {
    40% {
      transform: rotate(-10deg);
    }
    70% {
      transform: rotate(4deg);
    }
  }
  @keyframes drift {
    0% {
      opacity: 0;
      transform: translate(0, 2px);
    }
    40% {
      opacity: 1;
    }
    100% {
      opacity: 0;
      transform: translate(1.5px, -3px);
    }
  }
</style>
