// UI state. The window holds no engineering state of its own: everything
// here is either a projection of what the Rust side reported, or view
// state (what's selected, what's open). Refreshes are pulled, never pushed:
// a "changed" signal bumps `tick`, and whoever is on screen re-reads.

import { api, errorText, native, onChanged } from "./api";
import type { Overview, TaskView } from "./types";

export type View =
  | { kind: "home" }
  | { kind: "task"; id: string }
  | { kind: "rules" }
  | { kind: "file"; path: string; line?: number }
  | { kind: "diff"; run: string; path: string };

export type Overlay = null | "palette" | "help" | "new" | "files";

interface Prefs {
  vim: boolean;
  agent: string;
  policy: "ask" | "auto";
  showDone: boolean;
}

const PREFS_KEY = "kitsu.prefs";

function loadPrefs(): Prefs {
  const d: Prefs = { vim: true, agent: "claude", policy: "ask", showDone: false };
  try {
    return { ...d, ...JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") };
  } catch {
    return d;
  }
}

class App {
  overview = $state<Overview | null>(null);
  loadError = $state<string | null>(null);
  view = $state<View>({ kind: "home" });
  back: View[] = [];
  overlay = $state<Overlay>(null);
  selected = $state<string | null>(null);
  tick = $state(0);
  toast = $state<{ text: string; tone: "ok" | "bad" | "info" } | null>(null);
  prefs = $state<Prefs>(loadPrefs());
  preview = !native;
  private refreshing = false;
  private again = false;
  private toastTimer: ReturnType<typeof setTimeout> | undefined;

  async start() {
    try {
      await api.openRepo();
      await this.refresh();
    } catch (e) {
      this.loadError = errorText(e);
    }
    await onChanged(() => this.changed());
  }

  /** Coalesces bursts: at most one overview fetch in flight, one queued. */
  async refresh() {
    if (this.refreshing) {
      this.again = true;
      return;
    }
    this.refreshing = true;
    try {
      this.overview = await api.overview();
      this.loadError = null;
      if (!this.selected && this.overview.tasks.length) this.selected = this.overview.tasks[0]!.id;
    } catch (e) {
      this.loadError = errorText(e);
    } finally {
      this.refreshing = false;
      if (this.again) {
        this.again = false;
        void this.refresh();
      }
    }
  }

  changed() {
    this.tick++;
    void this.refresh();
  }

  savePrefs() {
    try {
      localStorage.setItem(PREFS_KEY, JSON.stringify(this.prefs));
    } catch {
      // Private mode or storage off: preferences just don't persist.
    }
  }

  go(v: View) {
    if (JSON.stringify(v) === JSON.stringify(this.view)) return;
    this.back.push(this.view);
    if (this.back.length > 50) this.back.shift();
    this.view = v;
    if (v.kind === "task") this.selected = v.id;
  }

  goBack() {
    const v = this.back.pop();
    if (v) this.view = v;
    else this.view = { kind: "home" };
  }

  openTask(id: string) {
    this.go({ kind: "task", id });
  }

  notify(text: string, tone: "ok" | "bad" | "info" = "info") {
    this.toast = { text, tone };
    clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => (this.toast = null), tone === "bad" ? 7000 : 3500);
  }

  /** Visible task list in rail order, respecting the Done toggle. */
  visibleTasks(filter = ""): TaskView[] {
    const all = this.overview?.tasks ?? [];
    const f = filter.trim().toLowerCase();
    return all.filter((t) => (this.prefs.showDone || t.attention !== "quiet") && (!f || t.title.toLowerCase().includes(f) || t.id.includes(f)));
  }

  task(id: string): TaskView | undefined {
    return this.overview?.tasks.find((t) => t.id === id);
  }
}

export const app = new App();
