<script lang="ts">
  // Coach marks. A spotlight around the step's target and a small popover
  // beside it. Nothing here blocks the page: the point is to do the thing
  // being described, so clicks and keys go through to the app underneath.
  import { onMount, untrack } from "svelte";
  import { t } from "../lib/i18n/index.svelte";
  import type { TourStep } from "../lib/tour.svelte";
  import Fox from "./Fox.svelte";

  let { steps, onclose, paused = false }: { steps: TourStep[]; onclose: () => void; paused?: boolean } = $props();

  interface Box {
    x: number;
    y: number;
    w: number;
    h: number;
  }

  let index = $state(0);
  let did = $state(false);
  // Only a step whose action wasn't already done when it opened moves on by
  // itself; otherwise going Back would bounce straight forward again.
  let armed = false;
  let rect = $state<Box | null>(null);
  let popW = $state(0);
  let popH = $state(0);
  let vw = $state(1200);
  let vh = $state(800);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let scrolledFor = -1;

  // The plan is fixed when the tour opens, so "3 of 8" doesn't turn into
  // "3 of 6" because accepting a run made the review steps moot.
  const plan = untrack(() => steps.map((s) => !s.when || s.when()));
  const step = $derived(steps[index]);
  const total = plan.filter(Boolean).length;
  const position = $derived(plan.slice(0, index + 1).filter(Boolean).length);
  const last = $derived(index === steps.length - 1);

  function go(i: number, dir: 1 | -1) {
    while (i >= 0 && i < steps.length && steps[i]!.when && !steps[i]!.when!()) i += dir;
    if (i >= steps.length) return onclose();
    if (i < 0) return;
    clearTimeout(timer);
    index = i;
    rect = null;
    const s = steps[i]!;
    s.enter?.();
    did = !!s.done?.();
    armed = !did;
  }

  export function next() {
    if (!step) return;
    if (step.done && !did) step.skip?.();
    go(index + 1, 1);
  }

  function back() {
    go(index - 1, -1);
  }

  onMount(() => {
    go(0, 1);
    // Targets move (pages load, views animate, panels open), so follow them
    // every frame rather than guessing which event to listen to.
    let raf = 0;
    const follow = () => {
      rect = measure();
      raf = requestAnimationFrame(follow);
    };
    follow();
    return () => {
      cancelAnimationFrame(raf);
      clearTimeout(timer);
    };
  });

  function measure(): Box | null {
    for (const name of step?.target ?? []) {
      const el = document.querySelector<HTMLElement>(`[data-tour="${name}"]`);
      const r = el?.getBoundingClientRect();
      if (!el || !r || r.width === 0 || r.height === 0) continue;
      if (scrolledFor !== index) {
        scrolledFor = index;
        el.scrollIntoView({ block: "nearest", behavior: "smooth" });
      }
      const pad = 6;
      const box = { x: r.left - pad, y: r.top - pad, w: r.width + pad * 2, h: r.height + pad * 2 };
      // Same box as last frame: keep the old object so nothing re-renders.
      if (rect && Math.abs(rect.x - box.x) < 0.5 && Math.abs(rect.y - box.y) < 0.5 && Math.abs(rect.w - box.w) < 0.5 && Math.abs(rect.h - box.h) < 0.5) return rect;
      return box;
    }
    return null;
  }

  // Did the user do what the step asks? Watched reactively: done() reads app state.
  $effect(() => {
    const s = step;
    if (!s?.done || !armed || !s.done()) return;
    did = true;
    armed = false;
    if (!s.stay) timer = setTimeout(() => go(index + 1, 1), 700);
  });

  const margin = 16;
  const gap = 14;
  const pos = $derived.by(() => {
    const w = popW || 340;
    const h = popH || 180;
    const clampX = (x: number) => Math.min(Math.max(x, margin), vw - w - margin);
    const clampY = (y: number) => Math.min(Math.max(y, margin), vh - h - margin);
    if (!rect) {
      if (step?.place === "center") return { x: (vw - w) / 2, y: (vh - h) / 2 };
      return { x: (vw - w) / 2, y: vh - h - 56 };
    }
    const r = rect;
    const fits = {
      right: r.x + r.w + gap + w <= vw - margin,
      left: r.x - gap - w >= margin,
      bottom: r.y + r.h + gap + h <= vh - margin,
      top: r.y - gap - h >= margin,
    };
    const at = {
      right: { x: r.x + r.w + gap, y: clampY(r.y) },
      left: { x: r.x - gap - w, y: clampY(r.y) },
      bottom: { x: clampX(r.x), y: r.y + r.h + gap },
      top: { x: clampX(r.x), y: r.y - gap - h },
    };
    const want = step?.place && step.place !== "center" ? step.place : "bottom";
    const order = [want, ...(["bottom", "right", "top", "left"] as const).filter((p) => p !== want)];
    const side = order.find((p) => fits[p]);
    // Nowhere fits (a target taller than the window): sit inside its lower edge.
    return side ? at[side] : { x: clampX(r.x + r.w - w), y: clampY(r.y + r.h - h - margin) };
  });

  function inEditable(el: EventTarget | null): boolean {
    const e = el as HTMLElement | null;
    return !!e && (e.tagName === "INPUT" || e.tagName === "TEXTAREA" || e.isContentEditable || !!e.closest(".cm-editor, [role=tree]"));
  }

  // Capture phase, so Esc ends the tour instead of also going back a page.
  function key(e: KeyboardEvent) {
    if (paused || e.metaKey || e.ctrlKey || e.altKey || inEditable(e.target)) return;
    if (e.key === "ArrowRight") next();
    else if (e.key === "ArrowLeft") back();
    else if (e.key === "Escape") onclose();
    else return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<svelte:window onkeydowncapture={key} bind:innerWidth={vw} bind:innerHeight={vh} />

{#if step}
  <div class="tour">
    {#if rect}
      <div class="spot" style:transform="translate({rect.x}px, {rect.y}px)" style:width="{rect.w}px" style:height="{rect.h}px"></div>
    {:else if step.dim}
      <div class="scrim"></div>
    {/if}

    {#key index}
      <div
        class="pop"
        role="dialog"
        aria-modal="false"
        aria-labelledby="tour-title"
        style:left="{pos.x}px"
        style:top="{pos.y}px"
        bind:offsetWidth={popW}
        bind:offsetHeight={popH}
      >
        <div class="head">
          <Fox state={did ? "happy" : (step.fox ?? "idle")} size={36} />
          <div class="heading">
            <span class="count">{t("tour.count", { step: position, total })}</span>
            <h2 id="tour-title">{step.title()}</h2>
          </div>
        </div>
        <p class="body" aria-live="polite">{@html step.body()}</p>
        {#if did && step.done}<p class="did">✓ {t("tour.did")}</p>{/if}
        <div class="foot">
          <ol class="dots" aria-hidden="true">
            {#each steps as s, i (s.id)}
              {#if plan[i]}<li class:on={i === index} class:past={i < index}></li>{/if}
            {/each}
          </ol>
          <span class="spacer"></span>
          {#if !last}<button class="btn quiet mini" onclick={onclose}>{t("tour.skip")}</button>{/if}
          {#if position > 1}<button class="btn mini" onclick={back}>{t("tour.back")}</button>{/if}
          <button class="btn mini" class:primary={!step.done || did || last} onclick={next}>{last ? t("tour.finish") : t("tour.next")}</button>
        </div>
        <p class="keys hint">{t("tour.keys")}</p>
      </div>
    {/key}
  </div>
{/if}

<style>
  .tour {
    position: fixed;
    inset: 0;
    z-index: 40;
    pointer-events: none;
  }
  .spot {
    position: absolute;
    top: 0;
    left: 0;
    border-radius: 12px;
    box-shadow:
      0 0 0 2px var(--accent),
      0 0 0 9999px var(--spot);
    transition:
      transform var(--quick) var(--ease),
      width var(--quick) var(--ease),
      height var(--quick) var(--ease);
    animation: fade-in var(--quick) var(--ease);
  }
  .scrim {
    position: absolute;
    inset: 0;
    background: var(--spot);
    animation: fade-in var(--quick) var(--ease);
  }
  .pop {
    position: absolute;
    width: min(340px, calc(100vw - 32px));
    padding: var(--s4) var(--s4) var(--s3);
    border-radius: 14px;
    background: var(--elev);
    border: 1px solid var(--line);
    box-shadow: var(--shadow);
    pointer-events: auto;
    transition:
      left var(--quick) var(--ease),
      top var(--quick) var(--ease);
    animation: pop-in var(--quick) var(--ease);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--s3);
    margin-bottom: var(--s2);
  }
  .heading {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .count {
    color: var(--faint);
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 650;
    line-height: 1.3;
  }
  .body {
    margin: 0;
    color: var(--muted);
    font-size: 13.5px;
    line-height: 1.6;
  }
  .body :global(kbd) {
    color: var(--text);
  }
  .did {
    margin: var(--s2) 0 0;
    color: var(--ok);
    font-size: 13px;
    font-weight: 500;
    animation: enter var(--quick) var(--ease);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--s2);
    margin-top: var(--s4);
  }
  .spacer {
    flex: 1;
  }
  .mini {
    height: 28px;
    padding: 0 var(--s3);
    font-size: 13px;
  }
  .dots {
    display: flex;
    gap: 5px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .dots li {
    width: 6px;
    height: 6px;
    border-radius: 3px;
    background: var(--line);
    transition:
      width var(--quick) var(--ease),
      background-color var(--quick) var(--ease);
  }
  .dots li.past {
    background: var(--faint);
  }
  .dots li.on {
    width: 16px;
    background: var(--accent);
  }
  .keys {
    margin: var(--s2) 0 0;
    font-size: 11.5px;
  }
</style>
