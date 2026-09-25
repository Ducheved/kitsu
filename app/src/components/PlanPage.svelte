<script lang="ts">
  // The Plan: every task as a block, every `after` as an arrow from the
  // prerequisite to the task that waits for it. Editing here writes the
  // task files (typed commands, only the `after`/`checks`/`scope` keys);
  // where the blocks sit is view state, kept per repository in local prefs.
  import { onMount, tick } from "svelte";
  import { app } from "../lib/app.svelte";
  import { errorText } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import {
    BLOCK_H,
    BLOCK_W,
    type Edge,
    type Placed,
    type Point,
    autoLayout,
    bounds,
    edgeKey,
    edgeMid,
    edgePath,
    edges as edgesOf,
    inPort,
    loadPlaced,
    neighbour,
    outPort,
    savePlaced,
    wouldCycle,
  } from "../lib/plan";
  import { statusText } from "../lib/status";
  import type { Attention, Plan, PlanTask, TaskView } from "../lib/types";
  import Fox from "./Fox.svelte";
  import Glyph from "./Glyph.svelte";
  // This project's commands, fixed for as long as the component lives.
  const api = app.api;

  const MIN_K = 0.2;
  const MAX_K = 2;
  const ORDER: Attention[] = ["needs_you", "working", "ready", "waiting", "quiet"];
  const WORD: Record<Attention, "rail.needsYou" | "rail.working" | "rail.ready" | "rail.waiting" | "rail.done"> = {
    needs_you: "rail.needsYou",
    working: "rail.working",
    ready: "rail.ready",
    waiting: "rail.waiting",
    quiet: "rail.done",
  };

  let plan = $state<Plan | null>(null);
  let loadError = $state<string | null>(null);
  let root = $state("");
  let placed = $state<Placed>({});

  let vw = $state(0);
  let vh = $state(0);
  let pan = $state<Point>({ x: 0, y: 0 });
  let k = $state(1);
  /** Eases pan and zoom for key presses and Fit, not for dragging. */
  let glide = $state(false);
  let fitted = false;

  let selBlock = $state<string | null>(null);
  let selEdge = $state<string | null>(null);
  let hoverEdge = $state<string | null>(null);
  let panel = $state(false);

  type Gesture =
    | { kind: "pending-pan"; sx: number; sy: number; pan: Point; id: number }
    | { kind: "pan"; sx: number; sy: number; pan: Point }
    | { kind: "pending-drag"; id: string; sx: number; sy: number; off: Point; pointer: number }
    | { kind: "drag"; id: string; off: Point }
    | { kind: "connect"; from: string; at: Point; over: string | null };
  let gesture = $state<Gesture | null>(null);

  let creating = $state<{ at: Point; title: string; after: string[] } | null>(null);
  let message = $state<{ text: string; tone: "ok" | "bad" | "info"; undo?: boolean } | null>(null);
  let lastEdit = $state<{ id: string; after: string[] } | null>(null);
  let messageTimer: ReturnType<typeof setTimeout> | undefined;

  let viewport: HTMLDivElement | undefined = $state();
  let createInput: HTMLInputElement | undefined = $state();

  // Data --------------------------------------------------------------------

  $effect(() => {
    void app.tick;
    void load();
  });

  async function load() {
    try {
      const p = await api.plan();
      // An edit in flight already updated the local copy; the fresh read
      // wins either way, it's what's on disk.
      plan = p;
      loadError = null;
      if (selBlock && !p.tasks.some((x) => x.id === selBlock)) selBlock = null;
      await tick();
      if (!fitted && vw > 0 && p.tasks.length) {
        fitted = true;
        fit(false);
      }
    } catch (e) {
      loadError = errorText(e);
    }
  }

  $effect(() => {
    const r = app.overview?.repo.root ?? "";
    if (r !== root) {
      root = r;
      placed = loadPlaced(r);
    }
  });

  const tasks = $derived(plan?.tasks ?? []);
  const byId = $derived(new Map(tasks.map((x) => [x.id, x])));
  const viewOf = (id: string): TaskView | undefined => app.task(id);
  const attentionOf = (x: PlanTask): Attention => viewOf(x.id)?.attention ?? (x.state === "open" ? "ready" : "quiet");
  const rank = (x: PlanTask) => ORDER.indexOf(attentionOf(x));
  const layout = $derived(autoLayout(tasks, rank));
  const auto = $derived(layout.pos);
  // A long link keeps its lane only while both ends sit where the layout put them.
  const lanesOf = (e: Edge) => (placed[e.from] || placed[e.to] ? [] : (layout.lanes.get(edgeKey(e)) ?? []));
  const pos = $derived.by(() => {
    const m = new Map<string, Point>();
    for (const x of tasks) m.set(x.id, placed[x.id] ?? auto.get(x.id) ?? { x: 0, y: 0 });
    return m;
  });
  const links = $derived(edgesOf(tasks));
  const hand = $derived(tasks.some((x) => placed[x.id]));

  const at = (id: string) => pos.get(id) ?? { x: 0, y: 0 };
  const isDone = (id: string) => {
    const x = byId.get(id);
    return !!x && x.state !== "open";
  };

  // Geometry -------------------------------------------------------------------

  function toWorld(clientX: number, clientY: number): Point {
    const r = viewport?.getBoundingClientRect();
    return { x: (clientX - (r?.left ?? 0) - pan.x) / k, y: (clientY - (r?.top ?? 0) - pan.y) / k };
  }

  function hit(p: Point): string | null {
    // Topmost first: later blocks paint over earlier ones.
    for (let i = tasks.length - 1; i >= 0; i--) {
      const id = tasks[i]!.id;
      const q = at(id);
      if (p.x >= q.x && p.x <= q.x + BLOCK_W && p.y >= q.y && p.y <= q.y + BLOCK_H) return id;
    }
    return null;
  }

  function zoomAt(next: number, sx = vw / 2, sy = vh / 2) {
    const k2 = Math.min(MAX_K, Math.max(MIN_K, next));
    pan = { x: sx - ((sx - pan.x) * k2) / k, y: sy - ((sy - pan.y) * k2) / k };
    k = k2;
  }

  function eased(f: () => void) {
    glide = true;
    f();
    setTimeout(() => (glide = false), 220);
  }

  export function fit(animate = true) {
    const b = bounds(pos.values());
    if (!b || !vw || !vh) return;
    const pad = 56;
    const panelW = panel ? 340 : 0;
    const k2 = Math.min(1.1, Math.max(MIN_K, Math.min((vw - panelW - pad * 2) / b.w, (vh - 120 - pad) / b.h)));
    const go = () => {
      k = k2;
      pan = { x: (vw - panelW - b.w * k2) / 2 - b.x * k2, y: 64 + (vh - 64 - 56 - b.h * k2) / 2 - b.y * k2 };
    };
    if (animate) eased(go);
    else go();
  }

  /** Pan just enough to bring a block into view. */
  function reveal(id: string) {
    const p = at(id);
    const m = 48;
    const right = vw - (panel ? 340 : 0);
    const x0 = p.x * k + pan.x;
    const y0 = p.y * k + pan.y;
    let dx = 0;
    let dy = 0;
    if (x0 < m) dx = m - x0;
    else if (x0 + BLOCK_W * k > right - m) dx = right - m - (x0 + BLOCK_W * k);
    if (y0 < 64) dy = 64 - y0;
    else if (y0 + BLOCK_H * k > vh - m) dy = vh - m - (y0 + BLOCK_H * k);
    if (dx || dy) eased(() => (pan = { x: pan.x + dx, y: pan.y + dy }));
  }

  // Selection -----------------------------------------------------------------

  function select(id: string | null) {
    selBlock = id;
    selEdge = null;
    if (id) app.selected = id;
  }

  // Messages ------------------------------------------------------------------

  function say(text: string, tone: "ok" | "bad" | "info" = "info", undo = false) {
    message = { text, tone, undo };
    clearTimeout(messageTimer);
    messageTimer = setTimeout(() => (message = null), tone === "bad" ? 7000 : 4500);
  }

  const name = (id: string) => byId.get(id)?.title ?? id;

  // Writes ------------------------------------------------------------------

  async function save(id: string, lists: { after?: string[]; checks?: string[]; scope?: string[] }): Promise<boolean> {
    const x = byId.get(id);
    if (!x) return false;
    const before = { after: x.after, checks: x.checks, scope: x.scope };
    if (lists.after) x.after = lists.after;
    if (lists.checks) x.checks = lists.checks;
    if (lists.scope) x.scope = lists.scope;
    try {
      x.version = await api.updateTask(id, $state.snapshot(lists), x.version);
      app.changed();
      return true;
    } catch (e) {
      Object.assign(x, before);
      say(errorText(e), "bad");
      app.changed();
      return false;
    }
  }

  /** Why `to` can't wait for `from`, or null when it can. */
  function refusal(from: string, to: string): string | null {
    if (from === to) return t("plan.self");
    if (byId.get(to)?.after.includes(from)) return t("plan.exists", { from: name(from), to: name(to) });
    const loop = wouldCycle(tasks, from, to);
    if (loop) return t("plan.cycle", { chain: loop.join(" → ") });
    return null;
  }

  async function link(from: string, to: string) {
    const why = refusal(from, to);
    if (why) {
      say(why, "bad");
      return;
    }
    const x = byId.get(to)!;
    const prev = [...x.after];
    if (await save(to, { after: [...x.after, from] })) {
      lastEdit = { id: to, after: prev };
      say(t("plan.linked", { from: name(from), to: name(to) }), "ok", true);
      selEdge = edgeKey({ from, to });
      selBlock = null;
    }
  }

  async function unlink(e: Edge) {
    const x = byId.get(e.to);
    if (!x) return;
    const prev = [...x.after];
    if (await save(e.to, { after: x.after.filter((a) => a !== e.from) })) {
      lastEdit = { id: e.to, after: prev };
      if (selEdge === edgeKey(e)) selEdge = null;
      hoverEdge = null;
      say(t("plan.unlinked", { from: name(e.from), to: name(e.to) }), "info", true);
    }
  }

  async function undo() {
    const u = lastEdit;
    if (!u) return;
    lastEdit = null;
    if (await save(u.id, { after: u.after })) say(t("plan.undone"), "info");
  }

  function startCreate(atPoint: Point, after: string[] = []) {
    creating = { at: { x: atPoint.x - BLOCK_W / 2, y: atPoint.y - BLOCK_H / 2 }, title: "", after };
    message = null;
    void tick().then(() => createInput?.focus());
  }

  async function create() {
    const c = creating;
    if (!c || !c.title.trim()) return;
    creating = null;
    try {
      // The same path as the New task sheet.
      const { id } = await api.newEntity("task", c.title, { after: [...c.after] });
      placed[id] = c.at;
      message = null;
      savePlaced(root, placed);
      await load();
      select(id);
      app.changed();
    } catch (e) {
      say(errorText(e), "bad");
    }
  }

  // Pointer ---------------------------------------------------------------------

  function onBackgroundDown(e: PointerEvent) {
    if (e.button !== 0 || creating) return;
    const el = e.target as HTMLElement;
    if (el.closest(".block, .overlay, .edge-x, .panel")) return;
    viewport?.focus({ preventScroll: true });
    gesture = { kind: "pending-pan", sx: e.clientX, sy: e.clientY, pan: { ...pan }, id: e.pointerId };
  }

  function onBlockDown(e: PointerEvent, id: string) {
    if (e.button !== 0) return;
    e.stopPropagation();
    viewport?.focus({ preventScroll: true });
    select(id);
    const w = toWorld(e.clientX, e.clientY);
    const p = at(id);
    gesture = { kind: "pending-drag", id, sx: e.clientX, sy: e.clientY, off: { x: w.x - p.x, y: w.y - p.y }, pointer: e.pointerId };
  }

  function onPortDown(e: PointerEvent, id: string) {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    viewport?.focus({ preventScroll: true });
    viewport?.setPointerCapture(e.pointerId);
    select(id);
    gesture = { kind: "connect", from: id, at: toWorld(e.clientX, e.clientY), over: null };
  }

  function onMove(e: PointerEvent) {
    const g = gesture;
    if (!g) return;
    if (g.kind === "pending-pan" || g.kind === "pending-drag") {
      if (Math.hypot(e.clientX - g.sx, e.clientY - g.sy) < 4) return;
      viewport?.setPointerCapture(e.pointerId);
      gesture = g.kind === "pending-pan" ? { kind: "pan", sx: g.sx, sy: g.sy, pan: g.pan } : { kind: "drag", id: g.id, off: g.off };
      return onMove(e);
    }
    if (g.kind === "pan") {
      pan = { x: g.pan.x + e.clientX - g.sx, y: g.pan.y + e.clientY - g.sy };
    } else if (g.kind === "drag") {
      const w = toWorld(e.clientX, e.clientY);
      placed[g.id] = { x: Math.round(w.x - g.off.x), y: Math.round(w.y - g.off.y) };
    } else if (g.kind === "connect") {
      const w = toWorld(e.clientX, e.clientY);
      const over = hit(w);
      gesture = { ...g, at: w, over: over === g.from && !refusalFor(g.from, over) ? null : over };
    }
  }

  // Hovering the source itself isn't a target unless you mean it; a
  // refusal only shows on a real candidate.
  function refusalFor(from: string, over: string | null): string | null {
    return over && over !== from ? refusal(from, over) : null;
  }

  function onUp(e: PointerEvent) {
    const g = gesture;
    gesture = null;
    if (viewport?.hasPointerCapture(e.pointerId)) viewport.releasePointerCapture(e.pointerId);
    if (!g) return;
    if (g.kind === "pending-pan") {
      // A click on empty canvas clears the selection.
      if (!(e.target as HTMLElement).closest?.(".edge")) {
        selBlock = null;
        selEdge = null;
      }
    } else if (g.kind === "drag") {
      savePlaced(root, placed);
    } else if (g.kind === "connect") {
      const target = hit(toWorld(e.clientX, e.clientY));
      if (target && target !== g.from) void link(g.from, target);
      else if (!target) startCreate(toWorld(e.clientX, e.clientY), [g.from]);
    }
  }

  function onDblClick(e: MouseEvent) {
    const el = e.target as HTMLElement;
    if (el.closest(".block, .overlay, .edge-x, .panel, .edge")) return;
    startCreate(toWorld(e.clientX, e.clientY));
  }

  $effect(() => {
    const el = viewport;
    if (!el) return;
    // Not passive: the wheel pans and pinches the canvas, not the page.
    const wheel = (e: WheelEvent) => {
      if ((e.target as HTMLElement).closest(".panel, .overlay")) return;
      e.preventDefault();
      const r = el.getBoundingClientRect();
      if (e.ctrlKey || e.metaKey) zoomAt(k * Math.exp(-e.deltaY * 0.01), e.clientX - r.left, e.clientY - r.top);
      else pan = { x: pan.x - e.deltaX, y: pan.y - e.deltaY };
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  });

  onMount(() => {
    if (app.selected) selBlock = app.selected;
    viewport?.focus({ preventScroll: true });
  });

  // Keyboard (routed from App's window handler) ------------------------------

  const edgesOfSel = $derived(selBlock ? links.filter((e) => e.from === selBlock || e.to === selBlock) : []);

  function firstBlock(): string | null {
    let best: string | null = null;
    let s = Infinity;
    for (const [id, p] of pos) {
      const v = p.x + p.y * 0.5;
      if (v < s) {
        s = v;
        best = id;
      }
    }
    return best;
  }

  function moveSel(dir: "left" | "right" | "up" | "down") {
    const from = selBlock ?? (selEdge ? (dir === "left" ? selEdge.split(">")[0]! : selEdge.split(">")[1]!) : null);
    const next = from ? (selEdge && !selBlock ? from : neighbour(from, dir, pos, links)) : firstBlock();
    if (next) {
      select(next);
      reveal(next);
    }
  }

  /** Returns true when the key was the Plan's. */
  export function onKey(e: KeyboardEvent): boolean {
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === "z") {
      void undo();
      return true;
    }
    if (mod || e.altKey) return false;
    // Buttons in the panel and toolbar keep Enter, Space and Tab.
    const el = e.target as HTMLElement | null;
    if (el?.closest?.(".panel, .toolbar, .message") && (e.key === "Enter" || e.key === " " || e.key === "Tab")) return false;
    switch (e.key) {
      case "ArrowRight":
      case "l":
        moveSel("right");
        return true;
      case "ArrowLeft":
      case "h":
        moveSel("left");
        return true;
      case "ArrowUp":
      case "k":
        moveSel("up");
        return true;
      case "ArrowDown":
      case "j":
        moveSel("down");
        return true;
      case "Enter":
      case "o":
        if (selBlock) app.openTask(selBlock);
        else if (selEdge) app.openTask(selEdge.split(">")[1]!);
        return true;
      case "n":
        startCreate(toWorld((viewport?.getBoundingClientRect().left ?? 0) + (vw - (panel ? 340 : 0)) / 2, (viewport?.getBoundingClientRect().top ?? 0) + vh / 2));
        return true;
      case "e":
        if (!selBlock) select(firstBlock());
        panel = !!selBlock;
        if (selBlock) {
          const id = selBlock;
          void tick().then(() => reveal(id));
        }
        return true;
      case "Tab": {
        // Walk the selected block's links, so ⌫ can remove one by keyboard.
        const list = edgesOfSel.length ? edgesOfSel : selEdge ? links.filter((x) => edgeKey(x) === selEdge) : [];
        if (!list.length) return false;
        const i = list.findIndex((x) => edgeKey(x) === selEdge);
        const next = list[(i + (e.shiftKey ? list.length - 1 : 1) + (i < 0 && e.shiftKey ? 1 : 0)) % list.length]!;
        selEdge = edgeKey(next);
        return true;
      }
      case "Backspace":
      case "Delete": {
        const edge = links.find((x) => edgeKey(x) === selEdge);
        if (edge) {
          const back = edge.to;
          void unlink(edge).then(() => select(back));
        } else say(t("plan.pickLink"), "info");
        return true;
      }
      case "f":
        fit();
        return true;
      case "+":
      case "=":
        eased(() => zoomAt(k * 1.2));
        return true;
      case "-":
      case "_":
        eased(() => zoomAt(k / 1.2));
        return true;
      case "0":
        eased(() => zoomAt(1));
        return true;
      case "d":
        toggleDim();
        return true;
      case "Escape":
        if (creating) creating = null;
        else if (gesture) gesture = null;
        else if (panel) panel = false;
        else if (selEdge) selEdge = null;
        else return false;
        return true;
    }
    return false;
  }

  function toggleDim() {
    app.prefs.planDimDone = !app.prefs.planDimDone;
    app.savePrefs();
  }

  function tidy() {
    placed = {};
    savePlaced(root, placed);
    setTimeout(() => fit(), 0);
  }

  // Side panel --------------------------------------------------------------

  const current = $derived(selBlock ? byId.get(selBlock) : undefined);
  const neededBy = $derived(current ? tasks.filter((x) => x.after.includes(current.id)) : []);
  let scopeDraft = $state("");

  async function toggleCheck(c: string) {
    if (!current) return;
    const next = current.checks.includes(c) ? current.checks.filter((x) => x !== c) : [...current.checks, c];
    await save(current.id, { checks: next });
  }

  async function addScope() {
    if (!current) return;
    const add = scopeDraft
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s && !current.scope.includes(s));
    if (!add.length) return;
    if (await save(current.id, { scope: [...current.scope, ...add] })) scopeDraft = "";
  }

  async function removeScope(g: string) {
    if (current) await save(current.id, { scope: current.scope.filter((x) => x !== g) });
  }

  // Rendering helpers -------------------------------------------------------

  const live = $derived.by(() => {
    const g = gesture;
    if (!g || g.kind !== "connect") return null;
    const a = outPort(at(g.from));
    const target = g.over && g.over !== g.from ? g.over : null;
    const b = target ? inPort(at(target)) : g.at;
    const why = refusalFor(g.from, target);
    const loop = why && target ? wouldCycle(tasks, g.from, target) : null;
    return { d: edgePath(a, b), bad: !!why, why, loop: loop && loop.length > 2 ? loop : null, over: target, tip: g.at };
  });

  const pct = $derived(Math.round(k * 100));
  const toScreen = (p: Point) => ({ x: p.x * k + pan.x, y: p.y * k + pan.y });
  const svgBox = $derived.by(() => {
    const b = bounds(pos.values()) ?? { x: 0, y: 0, w: BLOCK_W, h: BLOCK_H };
    const m = 400;
    return { x: b.x - m, y: b.y - m, w: b.w + 2 * m, h: b.h + 2 * m };
  });

  function tone(a: Attention, v: TaskView | undefined): string {
    const failing = v && (v.status.kind === "failed" || v.status.kind === "interrupted" || (v.status.kind === "review" && v.status.verdict === "failing"));
    if (a === "needs_you") return failing ? "bad" : "warn";
    return a === "working" ? "work" : a === "ready" ? "ready" : a === "waiting" ? "wait" : "quiet";
  }
</script>

<section class="plan" aria-label={t("plan.title")}>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    class="viewport"
    class:panning={gesture?.kind === "pan"}
    class:connecting={gesture?.kind === "connect"}
    bind:this={viewport}
    bind:clientWidth={vw}
    bind:clientHeight={vh}
    tabindex="0"
    role="application"
    aria-label={t("plan.canvas")}
    aria-roledescription={t("plan.canvas")}
    onpointerdown={onBackgroundDown}
    onpointermove={onMove}
    onpointerup={onUp}
    onpointercancel={() => (gesture = null)}
    ondblclick={onDblClick}
  >
    <div class="grid" style:background-size="{24 * k}px {24 * k}px" style:background-position="{pan.x}px {pan.y}px"></div>

    <div class="world" class:glide style:transform="translate({pan.x}px, {pan.y}px) scale({k})">
      <svg
        class="edges"
        style:left="{svgBox.x}px"
        style:top="{svgBox.y}px"
        width={svgBox.w}
        height={svgBox.h}
        viewBox="{svgBox.x} {svgBox.y} {svgBox.w} {svgBox.h}"
        aria-hidden="true"
      >
        {#each links as e (edgeKey(e))}
          {@const a = outPort(at(e.from))}
          {@const b = inPort(at(e.to))}
          {@const key = edgeKey(e)}
          {@const d = edgePath(a, b, lanesOf(e))}
          <g
            class="edge"
            class:selected={selEdge === key}
            class:hover={hoverEdge === key}
            class:related={selBlock === e.from || selBlock === e.to}
            class:met={isDone(e.from)}
            class:dim={app.prefs.planDimDone && isDone(e.from) && isDone(e.to)}
          >
            <path
              class="hit"
              {d}
              role="presentation"
              onpointerenter={() => (hoverEdge = key)}
              onpointerleave={() => hoverEdge === key && (hoverEdge = null)}
              onclick={(ev) => {
                ev.stopPropagation();
                selEdge = key;
                selBlock = null;
                viewport?.focus({ preventScroll: true });
              }}
            ></path>
            <path class="line" {d}></path>
            <path class="head" d="M{b.x - 8},{b.y - 4.5} L{b.x - 1},{b.y} L{b.x - 8},{b.y + 4.5}"></path>
          </g>
        {/each}
      </svg>

      {#each tasks as task (task.id)}
        {@const p = at(task.id)}
        {@const v = viewOf(task.id)}
        {@const a = attentionOf(task)}
        <div
          class="block t-{tone(a, v)}"
          class:selected={selBlock === task.id}
          class:dragging={gesture?.kind === "drag" && gesture.id === task.id}
          class:dim={app.prefs.planDimDone && a === "quiet" && selBlock !== task.id}
          class:target-ok={live?.over === task.id && !live.bad}
          class:target-bad={live?.over === task.id && live.bad}
          class:source={gesture?.kind === "connect" && gesture.from === task.id}
          style:transform="translate({p.x}px, {p.y}px)"
          style:width="{BLOCK_W}px"
          style:height="{BLOCK_H}px"
          data-block={task.id}
          role="button"
          tabindex="-1"
          aria-pressed={selBlock === task.id}
          aria-label="{task.title}: {v ? statusText(v) : t(WORD[a])}"
          onpointerdown={(e) => onBlockDown(e, task.id)}
          ondblclick={() => app.openTask(task.id)}
        >
          <div class="head">
            {#if v}<Glyph attention={v.attention} status={v.status} size={13} />{:else}<Glyph attention={a} size={13} />{/if}
            <span class="word">{t(WORD[a])}</span>
            <span class="id mono">{task.id}</span>
          </div>
          <div class="title">{task.title}</div>
          <div class="reason">{v ? statusText(v) : t(task.state === "dropped" ? "status.dropped" : "status.done")}</div>
          <div class="chips">
            {#each task.checks as c (c)}<span class="chip mono">{c}</span>{/each}
          </div>
          <span class="port in" aria-hidden="true"></span>
          <button class="port out" title={t("plan.out")} aria-label={t("plan.out")} tabindex="-1" onpointerdown={(e) => onPortDown(e, task.id)}></button>
        </div>
      {/each}

      {#if live}
        <!-- Above the blocks, so the link you're drawing is never hidden. -->
        <svg class="edges" style:left="{svgBox.x}px" style:top="{svgBox.y}px" width={svgBox.w} height={svgBox.h} viewBox="{svgBox.x} {svgBox.y} {svgBox.w} {svgBox.h}" aria-hidden="true">
          <path class="live" class:bad={live.bad} d={live.d}></path>
          {#if live.over}
            {@const b = inPort(at(live.over))}
            <circle class="live-end" class:bad={live.bad} cx={b.x} cy={b.y} r="5"></circle>
          {/if}
        </svg>
      {/if}

      {#each links as e (edgeKey(e))}
        {@const key = edgeKey(e)}
        {#if selEdge === key || hoverEdge === key}
          {@const m = edgeMid(outPort(at(e.from)), inPort(at(e.to)), lanesOf(e))}
          <button
            class="edge-x"
            style:transform="translate({m.x - 11}px, {m.y - 11}px) scale({1 / Math.max(k, 0.6)})"
            title={t("plan.removeLink")}
            aria-label="{t('plan.removeLink')}: {t('plan.linkLabel', { from: name(e.from), to: name(e.to) })}"
            onpointerenter={() => (hoverEdge = key)}
            onpointerdown={(ev) => ev.stopPropagation()}
            onclick={(ev) => {
              ev.stopPropagation();
              void unlink(e);
            }}>×</button
          >
        {/if}
      {/each}

      {#if creating}
        <div class="block creating" style:transform="translate({creating.at.x}px, {creating.at.y}px)" style:width="{BLOCK_W}px" style:height="{BLOCK_H}px">
          <input
            bind:this={createInput}
            class="create-input"
            placeholder={t("new.title")}
            bind:value={creating.title}
            onpointerdown={(e) => e.stopPropagation()}
            onkeydown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void create();
              } else if (e.key === "Escape") {
                e.stopPropagation();
                creating = null;
                viewport?.focus({ preventScroll: true });
              }
            }}
            onblur={() => {
              if (creating && !creating.title.trim()) creating = null;
            }}
          />
          <div class="create-hint hint">
            {#if creating.after.length}{t("plan.newWaits", { task: creating.after.join(", ") })} · {/if}{t("plan.newHint")}
          </div>
        </div>
      {/if}
    </div>

    {#if live?.why && live.over}
      {@const s = toScreen(live.tip)}
      <div class="tip" style:transform="translate({s.x + 16}px, {s.y + 16}px)" role="status">
        {#if live.loop}<strong>{t("plan.loopTip")}</strong><span class="mono">{live.loop.join(" → ")}</span>{:else}{live.why}{/if}
      </div>
    {/if}

    {#if plan && !tasks.length && !creating}
      <div class="empty overlay">
        <Fox state="sleeping" size={44} />
        <span>{t("plan.empty")}</span>
      </div>
    {/if}
    {#if loadError}
      <div class="empty overlay tone-bad">{loadError}</div>
    {/if}
  </div>

  <header class="toolbar overlay" class:shifted={panel && current}>
    <div class="heading">
      <h1>{t("plan.title")}</h1>
      <span class="hint">{t("plan.tasks", { n: tasks.length })} · {t("plan.links", { n: links.length })}</span>
    </div>
    <div class="tools">
      <button class="btn quiet sm" class:on={app.prefs.planDimDone} aria-pressed={app.prefs.planDimDone} onclick={toggleDim} title="d">{t("plan.dimDone")}</button>
      <button class="btn quiet sm" disabled={!hand} onclick={tidy} title={t("plan.tidyHint")}>{t("plan.tidy")}</button>
      <span class="sep"></span>
      <button class="btn quiet sm icon" onclick={() => eased(() => zoomAt(k / 1.2))} title="{t('plan.zoomOut')} (−)" aria-label={t("plan.zoomOut")}>−</button>
      <button class="btn quiet sm pct mono" onclick={() => eased(() => zoomAt(1))} title="{t('plan.zoomReset')} (0)">{pct}%</button>
      <button class="btn quiet sm icon" onclick={() => eased(() => zoomAt(k * 1.2))} title="{t('plan.zoomIn')} (+)" aria-label={t("plan.zoomIn")}>+</button>
      <button class="btn sm" onclick={() => fit()} title="f">{t("plan.fit")} <kbd>f</kbd></button>
    </div>
  </header>

  {#if message}
    {#key message}
      <div class="message overlay {message.tone}" class:shifted={panel && current} role="status">
        <span>{message.text}</span>
        {#if message.undo && lastEdit}<button class="btn quiet sm" onclick={() => void undo()}>{t("plan.undo")} <kbd>⌘Z</kbd></button>{/if}
      </div>
    {/key}
  {/if}

  <footer class="keys overlay hint" class:shifted={panel && current}>
    <span><span class="dot"></span> {t("plan.hintLink")}</span>
    <span>{t("plan.hintAdd")}</span>
    <span>{t("plan.hintRemove")} <kbd>⌫</kbd></span>
    <span><kbd>←</kbd><kbd>→</kbd> {t("plan.hintMove")}</span>
    <span><kbd>e</kbd> {t("plan.hintEdit")}</span>
  </footer>

  {#if panel && current}
    {@const v = viewOf(current.id)}
    {@const a = attentionOf(current)}
    <aside class="panel overlay" aria-label={current.title}>
      <div class="p-head">
        <span class="p-state t-{tone(a, v)}">
          {#if v}<Glyph attention={v.attention} status={v.status} />{/if}
          <span class="word">{t(WORD[a])}</span>
        </span>
        <button class="btn quiet sm icon" onclick={() => (panel = false)} aria-label={t("plan.close")} title="Esc">×</button>
      </div>
      <h2>{current.title}</h2>
      <div class="hint mono">{current.path}</div>
      {#if v}<p class="p-reason">{statusText(v)}</p>{/if}
      <button class="btn" onclick={() => app.openTask(current.id)}>{t("plan.open")} <kbd>⏎</kbd></button>

      <h3>{t("blocked.waitsFor")}</h3>
      {#if current.after.length}
        <ul class="deps">
          {#each current.after as d (d)}
            <li>
              <button class="dep press" onclick={() => (select(d), reveal(d))}><Glyph attention={viewOf(d)?.attention ?? "quiet"} status={viewOf(d)?.status} size={12} /><span>{name(d)}</span></button>
              <button class="x" aria-label={t("plan.removeLink")} title={t("plan.removeLink")} onclick={() => void unlink({ from: d, to: current.id })}>×</button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="hint">{t("plan.nothing")}</p>
      {/if}

      <h3>{t("plan.neededBy")}</h3>
      {#if neededBy.length}
        <ul class="deps">
          {#each neededBy as d (d.id)}
            <li>
              <button class="dep press" onclick={() => (select(d.id), reveal(d.id))}><Glyph attention={viewOf(d.id)?.attention ?? "quiet"} status={viewOf(d.id)?.status} size={12} /><span>{d.title}</span></button>
              <button class="x" aria-label={t("plan.removeLink")} title={t("plan.removeLink")} onclick={() => void unlink({ from: current.id, to: d.id })}>×</button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="hint">{t("plan.nothing")}</p>
      {/if}

      <h3>{t("rules.checks")}</h3>
      {#if plan?.checks.length}
        <div class="toggles">
          {#each plan.checks as c (c)}
            <button class="toggle mono press" class:on={current.checks.includes(c)} aria-pressed={current.checks.includes(c)} onclick={() => void toggleCheck(c)}>
              <span class="box" aria-hidden="true">{current.checks.includes(c) ? "✓" : ""}</span>{c}
            </button>
          {/each}
        </div>
        <p class="hint">{t("plan.checksHint")}</p>
      {:else}
        <p class="hint">{t("plan.noChecks")}</p>
      {/if}

      <h3>{t("new.scope")}</h3>
      <div class="globs">
        {#each current.scope as g (g)}
          <span class="glob mono">{g}<button class="x" aria-label={t("plan.removeGlob", { glob: g })} onclick={() => void removeScope(g)}>×</button></span>
        {:else}
          <span class="hint">{t("rules.wholeRepo")}</span>
        {/each}
      </div>
      <input
        class="field mono"
        placeholder={t("plan.scopeAdd")}
        bind:value={scopeDraft}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void addScope();
          } else if (e.key === "Escape") {
            (e.currentTarget as HTMLInputElement).blur();
            viewport?.focus({ preventScroll: true });
          }
        }}
      />
    </aside>
  {/if}
</section>

<style>
  .plan {
    position: relative;
    height: 100%;
    overflow: hidden;
    background: var(--bg);
  }
  .viewport {
    position: absolute;
    inset: 0;
    overflow: hidden;
    cursor: default;
    touch-action: none;
    user-select: none;
    outline: none;
  }
  .viewport.panning {
    cursor: grabbing;
  }
  .viewport.connecting {
    cursor: crosshair;
  }
  .grid {
    position: absolute;
    inset: 0;
    pointer-events: none;
    background-image: radial-gradient(circle, var(--line) 1px, transparent 1.2px);
    opacity: 0.9;
  }
  .world {
    position: absolute;
    left: 0;
    top: 0;
    transform-origin: 0 0;
  }
  .world.glide {
    transition: transform 200ms var(--ease);
  }
  .edges {
    position: absolute;
    overflow: visible;
    pointer-events: none;
  }
  .edge .hit {
    fill: none;
    stroke: transparent;
    stroke-width: 14;
    pointer-events: stroke;
    cursor: pointer;
  }
  .edge .line {
    fill: none;
    stroke: var(--faint);
    stroke-width: 1.6;
    transition:
      stroke var(--fast) var(--ease),
      stroke-width var(--fast) var(--ease),
      opacity var(--fast) var(--ease);
  }
  .edge .head {
    fill: none;
    stroke: var(--faint);
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .edge.met .line {
    stroke-dasharray: 3 4;
  }
  .edge.dim {
    opacity: 0.45;
  }
  .edge.related .line,
  .edge.related .head {
    stroke: var(--muted);
    stroke-width: 2;
  }
  .edge.hover .line,
  .edge.hover .head,
  .edge.selected .line,
  .edge.selected .head {
    stroke: var(--accent);
    stroke-width: 2.2;
  }
  .live {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2;
    stroke-dasharray: 6 5;
    animation: march 0.6s linear infinite;
  }
  .live.bad {
    stroke: var(--bad);
  }
  @keyframes march {
    to {
      stroke-dashoffset: -11;
    }
  }

  .block {
    position: absolute;
    left: 0;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 10px 14px 10px 16px;
    border-radius: 12px;
    background: var(--elev);
    border: 1px solid var(--line);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.04);
    cursor: grab;
    transition:
      transform var(--quick) var(--ease),
      box-shadow var(--fast) var(--ease),
      border-color var(--fast) var(--ease),
      opacity var(--quick) var(--ease);
  }
  .block::before {
    content: "";
    position: absolute;
    left: 6px;
    top: 12px;
    bottom: 12px;
    width: 3px;
    border-radius: 2px;
    background: var(--bar, transparent);
  }
  .block:hover {
    border-color: color-mix(in srgb, var(--line) 50%, var(--faint));
  }
  .block.selected {
    border-color: var(--accent);
    box-shadow:
      0 0 0 3px var(--accent-soft),
      var(--shadow);
  }
  .block.dragging {
    transition: none;
    cursor: grabbing;
    box-shadow: var(--shadow);
    z-index: 2;
  }
  .block.dim {
    opacity: 0.5;
  }
  .block.target-ok {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .block.target-bad {
    border-color: var(--bad);
    border-style: dashed;
    box-shadow: 0 0 0 3px var(--bad-soft);
  }
  .t-warn {
    --bar: var(--warn);
    --word: var(--warn);
  }
  .t-bad {
    --bar: var(--bad);
    --word: var(--bad);
  }
  .t-work {
    --bar: var(--work);
    --word: var(--work);
  }
  .t-ready {
    --bar: var(--muted);
    --word: var(--text);
  }
  .t-wait {
    --word: var(--muted);
  }
  .t-quiet {
    --word: var(--faint);
  }
  .block > * {
    flex: none;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    height: 18px;
  }
  .word {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--word, var(--muted));
    white-space: nowrap;
  }
  .id {
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--faint);
    font-size: 10.5px;
  }
  .title {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    font-weight: 550;
    font-size: 13.5px;
    line-height: 18px;
  }
  .reason {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
    font-size: 12px;
    line-height: 16px;
  }
  .chips {
    display: flex;
    gap: 4px;
    margin-top: auto;
    overflow: hidden;
    min-height: 18px;
  }
  .chip {
    flex: none;
    height: 18px;
    padding: 0 6px;
    border-radius: 5px;
    background: var(--hover);
    color: var(--muted);
    font-size: 10.5px;
    line-height: 18px;
  }
  .port {
    position: absolute;
    top: 50%;
    width: 12px;
    height: 12px;
    margin-top: -6px;
    border-radius: 50%;
    background: var(--elev);
    border: 1.5px solid var(--faint);
  }
  .port.in {
    left: -6.5px;
    width: 8px;
    height: 8px;
    margin-top: -4px;
    border-color: var(--line);
    background: var(--bg);
  }
  .port.out {
    right: -7px;
    cursor: crosshair;
    opacity: 0;
    transition:
      opacity var(--fast) var(--ease),
      transform var(--fast) var(--ease),
      background-color var(--fast) var(--ease);
  }
  /* A generous invisible grab area around the dot. */
  .port.out::after {
    content: "";
    position: absolute;
    inset: -8px;
    border-radius: 50%;
  }
  .block:hover .port.out,
  .block.selected .port.out,
  .block.source .port.out,
  .viewport.connecting .port.out {
    opacity: 1;
  }
  .port.out:hover,
  .block.source .port.out {
    background: var(--accent);
    border-color: var(--accent);
    transform: scale(1.25);
  }
  .edge-x {
    position: absolute;
    left: 0;
    top: 0;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--elev);
    border: 1px solid var(--accent);
    color: var(--accent);
    font-size: 15px;
    line-height: 19px;
    box-shadow: var(--shadow);
    transform-origin: 11px 11px;
    animation: fade-in var(--fast) var(--ease);
    z-index: 3;
  }
  .edge-x:hover {
    background: var(--accent);
    color: var(--bg);
  }
  .creating {
    border-color: var(--accent);
    border-style: dashed;
    box-shadow: 0 0 0 3px var(--accent-soft);
    cursor: text;
    justify-content: center;
    gap: var(--s2);
    animation: fade-in var(--quick) var(--ease);
  }
  .create-input {
    width: 100%;
    border: none;
    outline: none;
    background: transparent;
    font-weight: 550;
    font-size: 14px;
  }
  .create-hint {
    font-size: 11.5px;
  }
  .tip {
    position: absolute;
    left: 0;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-width: 300px;
    padding: 6px 10px;
    border-radius: 8px;
    background: var(--elev);
    border: 1px solid color-mix(in srgb, var(--bad) 45%, transparent);
    color: var(--bad);
    font-size: 12.5px;
    box-shadow: var(--shadow);
    pointer-events: none;
    z-index: 5;
  }
  .tip strong {
    font-weight: 600;
  }
  .tip .mono {
    color: var(--muted);
    font-size: 11px;
  }
  .live-end {
    fill: var(--accent);
  }
  .live-end.bad {
    fill: var(--bad);
  }

  .overlay {
    position: absolute;
    z-index: 4;
  }
  .toolbar {
    top: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s4);
    padding: var(--s4) var(--s5) var(--s3);
    background: linear-gradient(var(--bg) 55%, transparent);
    pointer-events: none;
  }
  .toolbar > * {
    pointer-events: auto;
  }
  .toolbar.shifted {
    right: 340px;
  }
  .heading {
    display: flex;
    align-items: baseline;
    gap: var(--s3);
  }
  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 650;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 3px;
    border-radius: 11px;
    background: var(--elev);
    border: 1px solid var(--line);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.04);
  }
  .sm {
    height: 28px;
    padding: 0 10px;
    font-size: 12.5px;
  }
  .sm.icon {
    width: 28px;
    padding: 0;
    justify-content: center;
    font-size: 16px;
  }
  .pct {
    min-width: 52px;
    justify-content: center;
    font-size: 12px;
  }
  .btn.on {
    color: var(--text);
    background: var(--active);
  }
  .sep {
    width: 1px;
    height: 18px;
    margin: 0 4px;
    background: var(--line);
  }
  .message {
    bottom: 52px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: var(--s3);
    max-width: min(620px, 80%);
    padding: 7px 8px 7px 14px;
    border-radius: 10px;
    background: var(--elev);
    border: 1px solid var(--line);
    box-shadow: var(--shadow);
    font-size: 13px;
    animation: message-in var(--quick) var(--ease);
  }
  @keyframes message-in {
    from {
      opacity: 0;
      transform: translate(-50%, 6px);
    }
  }
  .message.shifted {
    left: calc(50% - 170px);
  }
  .message.bad {
    background: var(--bad-soft);
    border-color: color-mix(in srgb, var(--bad) 35%, transparent);
    color: var(--bad);
    padding-right: 14px;
  }
  .message.ok {
    color: var(--text);
  }
  .keys {
    left: var(--s5);
    bottom: var(--s4);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px var(--s4);
    max-width: calc(100% - 2 * var(--s5));
    pointer-events: none;
  }
  .keys.shifted {
    max-width: calc(100% - 340px - 2 * var(--s5));
  }
  .keys kbd {
    margin-right: 2px;
  }
  .dot {
    display: inline-block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 1.5px solid var(--faint);
    vertical-align: -1px;
  }
  .empty {
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s3);
    color: var(--muted);
    pointer-events: none;
  }

  .panel {
    top: 0;
    right: 0;
    bottom: 0;
    width: 340px;
    display: flex;
    flex-direction: column;
    gap: var(--s2);
    padding: var(--s5);
    overflow: auto;
    background: var(--elev);
    border-left: 1px solid var(--line);
    box-shadow: -12px 0 32px rgba(0, 0, 0, 0.06);
    animation: slide-in var(--quick) var(--ease);
    scrollbar-width: thin;
  }
  @keyframes slide-in {
    from {
      opacity: 0;
      transform: translateX(16px);
    }
  }
  .p-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .p-state {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .panel h2 {
    margin: var(--s1) 0 0;
    font-size: 17px;
    line-height: 1.35;
  }
  .p-reason {
    margin: 0 0 var(--s2);
    color: var(--muted);
    font-size: 13px;
  }
  .panel > .btn {
    align-self: flex-start;
  }
  .panel h3 {
    margin: var(--s4) 0 0;
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--faint);
  }
  .panel p.hint {
    margin: 0;
  }
  .deps {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .deps li {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .dep {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 5px 8px;
    border-radius: 8px;
    text-align: left;
    font-size: 13px;
  }
  .dep span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dep:hover {
    background: var(--hover);
  }
  .x {
    flex: none;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    color: var(--faint);
    font-size: 15px;
    line-height: 20px;
  }
  .x:hover {
    color: var(--bad);
    background: var(--bad-soft);
  }
  .toggles {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 10px 0 7px;
    border-radius: 8px;
    border: 1px solid var(--line);
    color: var(--muted);
    font-size: 12px;
  }
  .toggle .box {
    display: inline-grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border-radius: 4px;
    border: 1.5px solid var(--faint);
    font-size: 10px;
    line-height: 1;
  }
  .toggle.on {
    border-color: var(--accent);
    background: var(--accent-soft);
    color: var(--text);
  }
  .toggle.on .box {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--bg);
  }
  .globs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    min-height: 24px;
    align-items: center;
  }
  .glob {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    height: 24px;
    padding: 0 2px 0 8px;
    border-radius: 6px;
    background: var(--hover);
    font-size: 12px;
  }
  .glob .x {
    width: 18px;
    height: 18px;
    line-height: 16px;
    font-size: 13px;
  }
  .panel .field {
    margin-top: 4px;
    font-size: 12.5px;
  }
</style>
