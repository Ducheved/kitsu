// The project switcher's rules: which project opens, what a key picks,
// where you land when the open one leaves the list, and that each project
// keeps its own view state while you're elsewhere.
import assert from "node:assert/strict";
import { test } from "node:test";
import { ViewMemory, afterRemoval, digitOf, moved, needsElsewhere, pickStart, projectAtKey, shortPath, step, tildify } from "../src/lib/repos.ts";

const list = [
  { id: "pay", needs_you: 2 },
  { id: "web", needs_you: 1 },
  { id: "infra", needs_you: 0 },
];

test("start opens where you launched, else where you were, else the first", () => {
  assert.equal(pickStart(list, "web", "infra"), "web");
  assert.equal(pickStart(list, null, "infra"), "infra");
  // Remembered one was removed from the list since: don't open a ghost.
  assert.equal(pickStart(list, null, "gone"), "pay");
  assert.equal(pickStart(list, "gone", null), "pay");
  assert.equal(pickStart([], "web", "web"), null);
});

test("1-9 pick in list order; anything else picks nothing", () => {
  assert.equal(projectAtKey(list, "1"), "pay");
  assert.equal(projectAtKey(list, "3"), "infra");
  assert.equal(projectAtKey(list, "4"), null);
  for (const k of ["0", "10", "a", "", "Enter"]) assert.equal(projectAtKey(list, k), null, k);
});

test("⌥digit counts by key code, since ⌥ on a Mac types another character", () => {
  assert.equal(digitOf({ key: "¡", code: "Digit1" }), "1");
  assert.equal(digitOf({ key: "™", code: "Digit2" }), "2");
  assert.equal(digitOf({ key: "3" }), "3");
  assert.equal(digitOf({ key: "0", code: "Digit0" }), null);
  assert.equal(digitOf({ key: "a", code: "KeyA" }), null);
});

test("removing the open project moves to the next one down, else up, else none", () => {
  assert.equal(afterRemoval(list, "pay", "pay"), "web");
  assert.equal(afterRemoval(list, "web", "web"), "infra");
  assert.equal(afterRemoval(list, "infra", "infra"), "web");
  assert.equal(afterRemoval([{ id: "solo" }], "solo", "solo"), null);
  // Removing another project doesn't move you.
  assert.equal(afterRemoval(list, "infra", "pay"), "pay");
});

test("the rail badge counts what needs you everywhere but here", () => {
  assert.equal(needsElsewhere(list, "pay"), 1);
  assert.equal(needsElsewhere(list, "infra"), 3);
  assert.equal(needsElsewhere(list, null), 3);
});

test("the highlight wraps, and reordering stops at the ends", () => {
  assert.equal(step(0, -1, 3), 2);
  assert.equal(step(2, 1, 3), 0);
  assert.equal(step(0, 1, 0), 0);
  assert.equal(moved(list, "pay", -1), null);
  assert.equal(moved(list, "pay", 1), 1);
  assert.equal(moved(list, "infra", 1), null);
  assert.equal(moved(list, "nope", 1), null);
});

test("each project keeps its own view state across a switch", () => {
  const m = new ViewMemory<{ selected: string; view: string }>();
  m.leave("pay", { selected: "bounded-retries", view: "task" });
  m.leave("web", { selected: "checkout-a11y", view: "plan" });
  assert.deepEqual(m.enter("pay"), { selected: "bounded-retries", view: "task" });
  assert.deepEqual(m.enter("web"), { selected: "checkout-a11y", view: "plan" });
  assert.equal(m.enter("infra"), undefined, "a project you haven't visited starts fresh");
  m.forget("web");
  assert.equal(m.enter("web"), undefined, "a removed project doesn't come back with old state");
});

test("view memory is bounded, least recently left goes first", () => {
  const m = new ViewMemory<number>(2);
  m.leave("a", 1);
  m.leave("b", 2);
  m.leave("a", 3);
  m.leave("c", 4);
  assert.equal(m.enter("b"), undefined);
  assert.equal(m.enter("a"), 3);
  assert.equal(m.enter("c"), 4);
});

test("paths read the way people say them", () => {
  assert.equal(tildify("/home/you/code/web"), "~/code/web");
  assert.equal(tildify("/Users/you"), "~");
  assert.equal(tildify("C:\\Users\\you\\code"), "~\\code");
  assert.equal(tildify("/srv/repo"), "/srv/repo");
  assert.equal(shortPath("/home/you/pay", "/home/you/pay"), "~/pay");
  assert.equal(shortPath("/home/you/pay/.git/kitsu/worktrees/r1", "/home/you/pay"), ".git/kitsu/worktrees/r1");
  assert.equal(shortPath("/home/you/pay-spike", "/home/you/pay"), "~/pay-spike");
});
