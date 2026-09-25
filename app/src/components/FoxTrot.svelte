<script lang="ts">
  // The fox seen from the side, trotting: small enough for the status bar,
  // where it keeps the "running" count company. Diagonal leg pairs swing
  // together, like a real trot. Still, it stands mid-stride.
  let { size = 16 }: { size?: number } = $props();
</script>

<svg class="trot" width={size * 1.5} height={size} viewBox="0 0 24 16" aria-hidden="true">
  <g class="body">
    <path class="tail" d="M7 8.2 C4.5 9.6 1.4 9 0.6 5.6 C2.6 6.8 4.8 6.6 7 6.6 Z" />
    <path class="tip" d="M0.6 5.6 C0.9 7.1 1.8 8.2 2.9 8.6 C2.4 7.6 2.2 6.6 2.4 6.2 C1.7 6.1 1.1 5.9 0.6 5.6 Z" />
    <path class="fur" d="M6.2 6.6 C9 5.3 13.4 5.2 16.4 6 L16.8 9.2 C13.8 10.1 9.2 10.1 6.6 9.5 Z" />
    <path class="fur" d="M15.4 5.8 L17.4 1.6 L18.8 4.3 L23.2 6.6 L18.6 8.8 L16 8.4 Z" />
    <path class="face" d="M18.6 8.8 L23.2 6.6 L19.6 6.6 Z" />
  </g>
  <g class="pair a">
    <line x1="15.2" y1="9.2" x2="16" y2="14.4" />
    <line x1="7.6" y1="9.4" x2="7" y2="14.4" />
  </g>
  <g class="pair b">
    <line x1="14" y1="9.4" x2="13.4" y2="14.4" />
    <line x1="8.8" y1="9.4" x2="9.6" y2="14.4" />
  </g>
</svg>

<style>
  .trot {
    display: inline-block;
    flex: none;
    overflow: visible;
    vertical-align: middle;
  }
  .trot * {
    transform-box: view-box;
  }
  .fur,
  .tail {
    fill: var(--accent);
  }
  .tip,
  .face {
    fill: var(--fox-face);
  }
  line {
    stroke: var(--accent);
    stroke-width: 1.5;
    stroke-linecap: round;
  }
  /* Hips sit on the body line; each pair swings about its own hips. */
  .pair {
    transform-origin: 11.5px 9.3px;
  }
  .pair.a {
    transform: skewX(-12deg);
    animation: stride 0.36s ease-in-out infinite alternate;
  }
  .pair.b {
    transform: skewX(12deg);
    animation: stride 0.36s ease-in-out infinite alternate-reverse;
  }
  .body {
    transform-origin: 12px 9px;
    animation: bounce 0.18s ease-in-out infinite alternate;
  }
  @keyframes stride {
    from {
      transform: skewX(-16deg);
    }
    to {
      transform: skewX(16deg);
    }
  }
  @keyframes bounce {
    to {
      transform: translateY(-0.5px);
    }
  }
</style>
