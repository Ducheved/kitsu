// Geometry and graph rules for the Plan view. Pure functions, no Svelte, so
// the layout and the cycle rule can be tested on their own (tests/plan.test.ts).
//
// Direction: an edge goes from a prerequisite to the task that waits for it
// (`after` in the waiting task's file), left to right on the canvas.

import type { PlanTask } from "./types";

export const BLOCK_W = 252;
export const BLOCK_H = 118;
const GAP_X = 96;
const GAP_Y = 28;

export interface Point {
  x: number;
  y: number;
}

export interface Edge {
  /** The prerequisite. */
  from: string;
  /** The task that waits for it. */
  to: string;
}

export const edgeKey = (e: Edge) => `${e.from}>${e.to}`;

/** Every `after` link whose both ends exist. */
export function edges(tasks: PlanTask[]): Edge[] {
  const ids = new Set(tasks.map((t) => t.id));
  const out: Edge[] = [];
  for (const t of tasks) for (const a of new Set(t.after)) if (ids.has(a) && a !== t.id) out.push({ from: a, to: t.id });
  return out;
}

/**
 * The loop that making `to` wait for `from` would close, as the chain of
 * ids in edge direction (`to → … → from → to`), or null if it's safe.
 * Waiting for yourself is the shortest loop.
 */
export function wouldCycle(tasks: PlanTask[], from: string, to: string): string[] | null {
  if (from === to) return [from, from];
  const after = new Map(tasks.map((t) => [t.id, t.after]));
  // Does `from` (transitively) wait for `to`? Walk its prerequisites.
  const prev = new Map<string, string>();
  const seen = new Set([from]);
  const queue = [from];
  while (queue.length) {
    const n = queue.shift()!;
    for (const d of after.get(n) ?? []) {
      if (seen.has(d)) continue;
      seen.add(d);
      prev.set(d, n);
      if (d === to) {
        // prev links point from a prerequisite to the task waiting for it.
        const chain = [to];
        let c = to;
        while (c !== from) {
          c = prev.get(c)!;
          chain.push(c);
        }
        chain.push(to);
        return chain;
      }
      queue.push(d);
    }
  }
  return null;
}

/** Where the automatic layout puts things. */
export interface Layout {
  pos: Map<string, Point>;
  /**
   * For a link that skips columns: the height at which it crosses each
   * column in between, through a gap kept free for it (key: `edgeKey`).
   */
  lanes: Map<string, Point[]>;
}

const LANE_H = 14;

/**
 * Layered layout (Sugiyama-style): a task sits one column right of its
 * latest prerequisite; a link that skips columns gets a thin placeholder
 * in each column it crosses, so it runs through a gap instead of under a
 * block. Each column is ordered by where its neighbours are (a few
 * barycenter sweeps), which untangles most crossings in plans of this
 * size, and columns are centered on the tallest one. Links that close a
 * loop are ignored for layering (the loop itself is reported by the Rust
 * side).
 */
export function autoLayout(tasks: PlanTask[], rank: (t: PlanTask) => number = () => 0): Layout {
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const layer = new Map<string, number>();
  const visiting = new Set<string>();
  const depth = (id: string): number => {
    const known = layer.get(id);
    if (known !== undefined) return known;
    if (visiting.has(id)) return 0;
    visiting.add(id);
    let d = 0;
    for (const a of byId.get(id)?.after ?? []) if (byId.has(a) && a !== id) d = Math.max(d, depth(a) + 1);
    visiting.delete(id);
    layer.set(id, d);
    return d;
  };
  for (const t of tasks) depth(t.id);

  const columns: string[][] = [];
  const initial = [...tasks].sort((a, b) => rank(a) - rank(b) || a.id.localeCompare(b.id));
  for (const t of initial) (columns[layer.get(t.id)!] ??= []).push(t.id);
  for (let i = 0; i < columns.length; i++) columns[i] ??= [];

  // Placeholders ("\0" never appears in a task id) chain long links.
  const preds = new Map<string, string[]>();
  const succs = new Map<string, string[]>();
  const connect = (a: string, b: string) => {
    (preds.get(b) ?? preds.set(b, []).get(b)!).push(a);
    (succs.get(a) ?? succs.set(a, []).get(a)!).push(b);
  };
  const chains = new Map<string, string[]>();
  for (const e of edges(tasks)) {
    const la = layer.get(e.from)!;
    const lb = layer.get(e.to)!;
    if (lb <= la) continue; // part of a loop
    let prev = e.from;
    const chain: string[] = [];
    for (let l = la + 1; l < lb; l++) {
      const d = `\0${edgeKey(e)}\0${l}`;
      columns[l]!.push(d);
      chain.push(d);
      connect(prev, d);
      prev = d;
    }
    connect(prev, e.to);
    if (chain.length) chains.set(edgeKey(e), chain);
  }

  const index = new Map<string, number>();
  columns.forEach((c) => c.forEach((id, i) => index.set(id, i)));
  const sweep = (col: string[], near: Map<string, string[]>) => {
    const score = new Map<string, number>();
    col.forEach((id, i) => {
      const n = (near.get(id) ?? []).map((x) => index.get(x)).filter((x): x is number => x !== undefined);
      score.set(id, n.length ? n.reduce((a, b) => a + b, 0) / n.length : i);
    });
    col.sort((a, b) => score.get(a)! - score.get(b)!);
    col.forEach((id, i) => index.set(id, i));
  };
  for (let pass = 0; pass < 6; pass++) {
    for (let c = 1; c < columns.length; c++) sweep(columns[c]!, preds);
    for (let c = columns.length - 2; c >= 0; c--) sweep(columns[c]!, succs);
  }

  const size = (id: string) => (id.startsWith("\0") ? LANE_H : BLOCK_H);
  const height = (col: string[]) => col.reduce((h, id) => h + size(id), 0) + Math.max(0, col.length - 1) * GAP_Y;
  const tallest = Math.max(BLOCK_H, ...columns.map(height));
  const pos = new Map<string, Point>();
  const lanePos = new Map<string, Point>();
  columns.forEach((col, c) => {
    let y = (tallest - height(col)) / 2;
    const x = c * (BLOCK_W + GAP_X);
    for (const id of col) {
      if (id.startsWith("\0")) lanePos.set(id, { x, y: y + LANE_H / 2 });
      else pos.set(id, { x, y });
      y += size(id) + GAP_Y;
    }
  });
  const lanes = new Map<string, Point[]>();
  for (const [k, chain] of chains) lanes.set(k, chain.map((d) => lanePos.get(d)!));
  return { pos, lanes };
}

/** A smooth left-to-right curve from a block's output to another's input. */
export function edgePath(a: Point, b: Point, lanes: Point[] = []): string {
  let d = `M${a.x},${a.y}`;
  let from = a;
  const curve = (to: Point) => {
    const dx = Math.max(48, Math.abs(to.x - from.x) / 2);
    d += ` C${from.x + dx},${from.y} ${to.x - dx},${to.y} ${to.x},${to.y}`;
  };
  // Through each gap: in on the left of the column, straight across.
  for (const l of lanes) {
    curve(l);
    d += ` L${l.x + BLOCK_W},${l.y}`;
    from = { x: l.x + BLOCK_W, y: l.y };
  }
  curve(b);
  return d;
}

/** Roughly halfway along `edgePath(a, b, lanes)`: where its remove button sits. */
export function edgeMid(a: Point, b: Point, lanes: Point[] = []): Point {
  if (lanes.length) {
    const l = lanes[Math.floor((lanes.length - 1) / 2)]!;
    return { x: l.x + BLOCK_W / 2, y: l.y };
  }
  const dx = Math.max(48, Math.abs(b.x - a.x) / 2);
  const x = 0.125 * a.x + 0.375 * (a.x + dx) + 0.375 * (b.x - dx) + 0.125 * b.x;
  return { x, y: (a.y + b.y) / 2 };
}

export const outPort = (p: Point): Point => ({ x: p.x + BLOCK_W, y: p.y + BLOCK_H / 2 });
export const inPort = (p: Point): Point => ({ x: p.x, y: p.y + BLOCK_H / 2 });

export function bounds(points: Iterable<Point>): { x: number; y: number; w: number; h: number } | null {
  let x0 = Infinity,
    y0 = Infinity,
    x1 = -Infinity,
    y1 = -Infinity;
  for (const p of points) {
    x0 = Math.min(x0, p.x);
    y0 = Math.min(y0, p.y);
    x1 = Math.max(x1, p.x + BLOCK_W);
    y1 = Math.max(y1, p.y + BLOCK_H);
  }
  return x0 === Infinity ? null : { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/**
 * The block to move to from `from` with an arrow key: right follows an edge
 * to a task that waits for it, left to a prerequisite, up and down to the
 * nearest block above or below. Among candidates, the one closest to the
 * direction wins.
 */
export function neighbour(
  from: string,
  dir: "left" | "right" | "up" | "down",
  pos: Map<string, Point>,
  links: Edge[],
): string | null {
  const p = pos.get(from);
  if (!p) return null;
  let candidates: string[];
  if (dir === "right") candidates = links.filter((e) => e.from === from).map((e) => e.to);
  else if (dir === "left") candidates = links.filter((e) => e.to === from).map((e) => e.from);
  else candidates = [...pos.keys()].filter((id) => id !== from && (dir === "up" ? pos.get(id)!.y < p.y - 1 : pos.get(id)!.y > p.y + 1));
  let best: string | null = null;
  let score = Infinity;
  for (const id of candidates) {
    const q = pos.get(id);
    if (!q) continue;
    const dx = q.x - p.x;
    const dy = q.y - p.y;
    // Moving along the axis is cheap, drifting off it costs more.
    const s = dir === "up" || dir === "down" ? Math.abs(dy) + 2.5 * Math.abs(dx) : Math.abs(dy) * 2 + Math.abs(dx) * 0.2;
    if (s < score) {
      score = s;
      best = id;
    }
  }
  return best;
}

/** Hand-placed blocks, per repository. View state, not intent: never in `.kitsu/`. */
export type Placed = Record<string, Point>;

const key = (root: string) => `kitsu.plan.positions:${root}`;

export function loadPlaced(root: string): Placed {
  try {
    const v = JSON.parse(localStorage.getItem(key(root)) ?? "{}");
    return v && typeof v === "object" ? (v as Placed) : {};
  } catch {
    return {};
  }
}

export function savePlaced(root: string, placed: Placed) {
  try {
    if (Object.keys(placed).length) localStorage.setItem(key(root), JSON.stringify(placed));
    else localStorage.removeItem(key(root));
  } catch {
    // Storage off: the layout just doesn't stick.
  }
}
