// The Plan view's graph rules: the layout puts every task right of what it
// waits for, and a link that would close a loop is caught before any write.
import assert from "node:assert/strict";
import { test } from "node:test";
import { BLOCK_H, BLOCK_W, autoLayout, edges, neighbour, wouldCycle } from "../src/lib/plan.ts";
import type { PlanTask } from "../src/lib/types.ts";

const task = (id: string, after: string[] = []): PlanTask => ({ id, title: id, state: "open", scope: [], checks: [], after, path: `.kitsu/tasks/${id}.md`, version: "v1" });

const chain = [task("a"), task("b", ["a"]), task("c", ["b"]), task("d", ["a", "c"]), task("e")];

test("every task sits right of what it waits for", () => {
  const pos = autoLayout(chain).pos;
  for (const e of edges(chain)) assert.ok(pos.get(e.from)!.x + BLOCK_W <= pos.get(e.to)!.x, `${e.from} -> ${e.to}`);
  assert.equal(pos.get("a")!.x, 0);
  assert.equal(pos.get("e")!.x, 0);
  // Longest path decides the column: d waits for c (column 2), so column 3.
  assert.ok(pos.get("d")!.x > pos.get("c")!.x);
});

test("no two blocks share a spot", () => {
  const many = Array.from({ length: 60 }, (_, i) => task(`t${i}`, i > 3 ? [`t${Math.floor(i / 3)}`] : []));
  const pos = autoLayout(many).pos;
  const spots = new Set([...pos.values()].map((p) => `${p.x},${p.y}`));
  assert.equal(spots.size, many.length);
});

test("a link that skips columns runs through a gap", () => {
  const { pos, lanes } = autoLayout(chain);
  // d waits for a (column 0) and sits in column 3: two lanes, in columns 1 and 2.
  const lane = lanes.get("a>d")!;
  assert.equal(lane.length, 2);
  assert.deepEqual(
    lane.map((p) => p.x),
    [pos.get("b")!.x, pos.get("c")!.x],
  );
  // The lane doesn't cross a block in its column.
  for (const p of lane)
    for (const [id, q] of pos) assert.ok(q.x !== p.x || p.y < q.y || p.y > q.y + BLOCK_H, `lane under ${id}`);
  assert.equal(lanes.get("a>b"), undefined);
});

test("a loop in the files doesn't hang the layout", () => {
  const loop = [task("a", ["b"]), task("b", ["a"])];
  assert.equal(autoLayout(loop).pos.size, 2);
});

test("links that would close a loop are refused, with the loop", () => {
  // c waits for b waits for a. Making a wait for c closes a -> b -> c -> a.
  assert.deepEqual(wouldCycle(chain, "c", "a"), ["a", "b", "c", "a"]);
  assert.deepEqual(wouldCycle(chain, "a", "a"), ["a", "a"]);
  assert.equal(wouldCycle(chain, "a", "e"), null);
  assert.equal(wouldCycle(chain, "e", "a"), null);
  // Already implied, not a loop.
  assert.equal(wouldCycle(chain, "a", "c"), null);
});

test("arrow keys follow the links", () => {
  const pos = autoLayout(chain).pos;
  const links = edges(chain);
  assert.equal(neighbour("a", "right", pos, links), "b");
  assert.equal(neighbour("c", "left", pos, links), "b");
  assert.equal(neighbour("e", "right", pos, links), null);
  // Up and down stay in the column when there's a block there.
  const [top, bottom] = pos.get("a")!.y < pos.get("e")!.y ? ["a", "e"] : ["e", "a"];
  assert.equal(neighbour(top!, "down", pos, links), bottom);
  assert.equal(neighbour(bottom!, "up", pos, links), top);
});
