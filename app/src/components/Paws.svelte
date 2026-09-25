<script lang="ts">
  // A trail of fox paw prints. Decoration only (aria-hidden, no pointer
  // events); the prints take the surrounding text color, so the caller
  // decides how faint they are. With `walk`, they appear one after another
  // in walking order and settle back; without motion they just sit there.
  let {
    count = 5,
    heading = 0,
    size = 12,
    walk = false,
    delay = 0,
  }: {
    count?: number;
    /** Direction of travel in degrees: 0 is up, 90 is right. */
    heading?: number;
    /** Height of one print in px. */
    size?: number;
    walk?: boolean;
    /** Before the first print appears, in ms. */
    delay?: number;
  } = $props();

  // One print is 10 units tall; strides and the gap between left and right
  // feet are in the same units. A fox puts its hind foot where the front one
  // was, so the trail is nearly a single line: long stride, narrow gait.
  const STRIDE = 13;
  const GAIT = 3.5;
  const PAD = 6;

  const prints = $derived.by(() => {
    const a = (heading * Math.PI) / 180;
    const dx = Math.sin(a);
    const dy = -Math.cos(a);
    return Array.from({ length: count }, (_, i) => {
      const side = i % 2 ? 1 : -1;
      const along = i * STRIDE;
      return { x: along * dx + (side * GAIT * -dy) / 2, y: along * dy + (side * GAIT * dx) / 2 };
    });
  });
  const box = $derived.by(() => {
    const xs = prints.map((p) => p.x);
    const ys = prints.map((p) => p.y);
    const x0 = Math.min(...xs) - PAD;
    const y0 = Math.min(...ys) - PAD;
    return { x0, y0, w: Math.max(...xs) + PAD - x0, h: Math.max(...ys) + PAD - y0 };
  });
  const scale = $derived(size / 10);
</script>

<svg class="paws" class:walk width={box.w * scale} height={box.h * scale} viewBox="{box.x0} {box.y0} {box.w} {box.h}" aria-hidden="true">
  {#each prints as p, i (i)}
    <g transform="translate({p.x} {p.y}) rotate({heading})">
      <g class="print" style:--i={i} style:--delay="{delay}ms">
        <!-- A fox print is an oval: small heel pad, four toes, the front two well ahead. -->
        <path d="M0 1.2 C1.7 1.2 2.4 2.7 1.9 3.7 C1.4 4.5 -1.4 4.5 -1.9 3.7 C-2.4 2.7 -1.7 1.2 0 1.2 Z" />
        <ellipse cx="-2.2" cy="-0.4" rx="0.8" ry="1.15" transform="rotate(-14 -2.2 -0.4)" />
        <ellipse cx="-0.8" cy="-2.9" rx="0.8" ry="1.2" />
        <ellipse cx="0.8" cy="-2.9" rx="0.8" ry="1.2" />
        <ellipse cx="2.2" cy="-0.4" rx="0.8" ry="1.15" transform="rotate(14 2.2 -0.4)" />
      </g>
    </g>
  {/each}
</svg>

<style>
  .paws {
    display: block;
    flex: none;
    overflow: visible;
    pointer-events: none;
  }
  .print {
    fill: currentColor;
    transform-box: fill-box;
    transform-origin: center;
  }
  /* Each print lands, glows faintly in the accent, and settles into the trail. */
  .walk .print {
    animation: step 1.5s var(--ease) backwards;
    animation-delay: calc(var(--delay) + var(--i) * 220ms);
  }
  @keyframes step {
    0% {
      opacity: 0;
      transform: scale(0.6);
    }
    18% {
      opacity: 1;
      transform: none;
      fill: var(--paw-walk, currentColor);
    }
    55% {
      fill: var(--paw-walk, currentColor);
    }
  }
</style>
