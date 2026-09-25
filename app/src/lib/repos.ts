// The project switcher's rules, kept apart from Svelte so they can be
// tested on their own: which project opens first, what a key picks, where
// to go when the open one leaves the list, and the view state each project
// keeps while you're elsewhere.

import type { ProjectSummary } from "./types";

type Listed = Pick<ProjectSummary, "id">;

/** The project to open at start: the one the app was started in, else the
 *  last one you had open (if it's still listed), else the first. */
export function pickStart(list: Listed[], launch: string | null, last: string | null): string | null {
  for (const id of [launch, last]) if (id && list.some((p) => p.id === id)) return id;
  return list[0]?.id ?? null;
}

/** `1`…`9` in the switcher, `⌥1`…`⌥9` anywhere: the Nth project in list order. */
export function projectAtKey(list: Listed[], key: string): string | null {
  if (!/^[1-9]$/.test(key)) return null;
  return list[Number(key) - 1]?.id ?? null;
}

/** The digit of a key event, from `code` so ⌥ on a Mac (which types ¡™£…)
 *  still counts: "Digit3" -> "3". */
export function digitOf(e: { key: string; code?: string }): string | null {
  const m = /^Digit([1-9])$/.exec(e.code ?? "");
  if (m) return m[1]!;
  return /^[1-9]$/.test(e.key) ? e.key : null;
}

/** Where to go when `removed` leaves the list: nowhere new unless it was the
 *  open one; then the next one down, else the one above, else nothing. */
export function afterRemoval(list: Listed[], removed: string, current: string | null): string | null {
  if (current !== removed) return current;
  const i = list.findIndex((p) => p.id === removed);
  const rest = list.filter((p) => p.id !== removed);
  if (!rest.length) return null;
  return rest[Math.min(Math.max(i, 0), rest.length - 1)]!.id;
}

/** Things that need you in projects other than the open one. */
export function needsElsewhere(list: Pick<ProjectSummary, "id" | "needs_you">[], current: string | null): number {
  return list.filter((p) => p.id !== current).reduce((n, p) => n + p.needs_you, 0);
}

/** A root as people say it: `/home/you/code/app` -> `~/code/app`. Only the
 *  usual home layouts; anything else is shown as is. */
export function tildify(root: string): string {
  return root.replace(/^\/(?:home|Users)\/[^/]+(?=\/|$)/, "~").replace(/^[A-Za-z]:\\Users\\[^\\]+(?=\\|$)/, "~");
}

/** `path` relative to `root` when it's inside it, else tildified (the root itself too). */
export function shortPath(path: string, root: string): string {
  if (path === root) return tildify(root);
  if (path.startsWith(root + "/")) return path.slice(root.length + 1);
  return tildify(path);
}

/** Moving the highlight in a list of `n`, wrapping at both ends. */
export function step(index: number, delta: number, n: number): number {
  if (n <= 0) return 0;
  return (((index + delta) % n) + n) % n;
}

/** `list` with `id` moved by `delta` places, for the ↑/↓ buttons; returns
 *  the new index, or null when it can't move. */
export function moved(list: Listed[], id: string, delta: number): number | null {
  const i = list.findIndex((p) => p.id === id);
  const to = i + delta;
  if (i < 0 || to < 0 || to >= list.length) return null;
  return to;
}

/**
 * View state per project, for as long as the window is open: what was
 * selected, which page, how you got there. Switching away files it,
 * switching back picks it up. Bounded, least recently used out first.
 */
export class ViewMemory<T> {
  private saved = new Map<string, T>();
  private limit: number;

  constructor(limit = 32) {
    this.limit = limit;
  }

  leave(id: string, state: T) {
    this.saved.delete(id);
    this.saved.set(id, state);
    while (this.saved.size > this.limit) this.saved.delete(this.saved.keys().next().value!);
  }

  enter(id: string): T | undefined {
    return this.saved.get(id);
  }

  forget(id: string) {
    this.saved.delete(id);
  }
}

