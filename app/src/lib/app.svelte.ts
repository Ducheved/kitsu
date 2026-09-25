// UI state. The window holds no engineering state of its own: everything
// here is either a projection of what the Rust side reported, or view
// state (what's selected, what's open). Refreshes are pulled, never pushed:
// a "changed" signal bumps `tick`, and whoever is on screen re-reads.

import { api, errorText, native, onChanged } from "./api";
import { i18n, type Locale, systemLocale } from "./i18n/index.svelte";
import type { Overview, TaskView } from "./types";

export type View =
  | { kind: "home" }
  | { kind: "task"; id: string }
  | { kind: "rules" }
  | { kind: "plan" }
  | { kind: "file"; path: string; line?: number }
  | { kind: "diff"; run: string; path: string };

export type Overlay = null | "palette" | "help" | "new" | "files" | "settings";
export type Theme = "system" | "light" | "dark";
export type Layout = "adaptive" | "work" | "code";
export type Mode = "work" | "code";

interface Prefs {
  vim: boolean;
  agent: string;
  policy: "ask" | "auto";
  showDone: boolean;
  lang: Locale | "system";
  theme: Theme;
  layout: Layout;
  tree: boolean;
  strip: boolean;
  /** Plan view: done tasks fade back. */
  planDimDone: boolean;
}

const PREFS_KEY = "kitsu.prefs";

function loadPrefs(): Prefs {
  const d: Prefs = { vim: true, agent: "claude", policy: "ask", showDone: false, lang: "system", theme: "system", layout: "adaptive", tree: true, strip: true, planDimDone: true };
  try {
    return { ...d, ...JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") };
  } catch {
    return d;
  }
}

const darkQuery = typeof matchMedia === "undefined" ? null : matchMedia("(prefers-color-scheme: dark)");

class App {
  overview = $state<Overview | null>(null);
  loadError = $state<string | null>(null);
  view = $state<View>({ kind: "home" });
  back: View[] = [];
  overlay = $state<Overlay>(null);
  selected = $state<string | null>(null);
  tick = $state(0);
  toast = $state<{ text: string; tone: "ok" | "bad" | "info"; cheer: boolean } | null>(null);
  /** Runs accepted in this window; the tour waits on it. */
  accepted = $state(0);
  prefs = $state<Prefs>(loadPrefs());
  /** Which layout is on screen. With the adaptive layout it follows what you open. */
  mode = $state<Mode>("work");
  dark = $state(false);
  preview = !native;
  private refreshing = false;
  private again = false;
  private toastTimer: ReturnType<typeof setTimeout> | undefined;

  async start() {
    this.mode = this.prefs.layout === "code" ? "code" : "work";
    this.applyTheme();
    darkQuery?.addEventListener("change", () => this.applyTheme());
    await i18n.use(this.prefs.lang === "system" ? systemLocale() : this.prefs.lang);
    try {
      await api.openRepo();
      await this.refresh();
    } catch (e) {
      this.loadError = errorText(e);
    }
    await onChanged(() => this.changed());
  }

  applyTheme() {
    const dark = this.prefs.theme === "dark" || (this.prefs.theme === "system" && !!darkQuery?.matches);
    this.dark = dark;
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }

  async setLang(lang: Locale | "system") {
    this.prefs.lang = lang;
    this.savePrefs();
    await i18n.use(lang === "system" ? systemLocale() : lang);
  }

  setTheme(theme: Theme) {
    this.prefs.theme = theme;
    this.savePrefs();
    this.applyTheme();
  }

  setLayout(layout: Layout) {
    this.prefs.layout = layout;
    this.savePrefs();
    if (layout !== "adaptive") this.mode = layout;
  }

  /** Toggle between the two layouts by hand. */
  setMode(mode: Mode) {
    this.mode = mode;
    if (this.prefs.layout !== "adaptive") this.setLayout(mode);
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
      // theme.js reads this before first paint.
      localStorage.setItem("kitsu.theme", this.prefs.theme);
    } catch {
      // Private mode or storage off: preferences just don't persist.
    }
  }

  go(v: View) {
    if (this.prefs.layout === "adaptive") this.mode = v.kind === "file" ? "code" : "work";
    if (JSON.stringify(v) === JSON.stringify(this.view)) return;
    this.back.push(this.view);
    if (this.back.length > 50) this.back.shift();
    this.view = v;
    if (v.kind === "task") this.selected = v.id;
  }

  goBack() {
    const v = this.back.pop();
    this.view = v ?? { kind: "home" };
    if (this.prefs.layout === "adaptive") this.mode = this.view.kind === "file" ? "code" : "work";
  }

  openTask(id: string) {
    this.go({ kind: "task", id });
  }

  /** `cheer` puts the happy fox on the toast: for accepts, not routine news. */
  notify(text: string, tone: "ok" | "bad" | "info" = "info", cheer = false) {
    this.toast = { text, tone, cheer };
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
