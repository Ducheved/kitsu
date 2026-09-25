// UI state. The window holds no engineering state of its own: everything
// here is either a projection of what the Rust side reported, or view
// state (what's selected, what's open). Refreshes are pulled, never pushed:
// a "changed" signal bumps `tick`, and whoever is on screen re-reads.
//
// Several projects can be open in one window. `repo` is the one on screen;
// each keeps its own view state while you're in another (see ViewMemory),
// and every call goes through an API bound to one project's id.

import { errorText, native, onChanged, projects, repoApi, type RepoApi } from "./api";
import { i18n, type Locale, systemLocale, t } from "./i18n/index.svelte";
import { ViewMemory, afterRemoval, pickStart } from "./repos";
import type { Overview, ProjectSummary, TaskView } from "./types";

export type View =
  | { kind: "home" }
  | { kind: "task"; id: string }
  | { kind: "rules" }
  | { kind: "plan" }
  | { kind: "tree" }
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
  /** Fox paw prints in the corner of the main pane. */
  paws: boolean;
  /** Plan view: done tasks fade back. */
  planDimDone: boolean;
}

/** What a project keeps while you're in another one. */
interface RepoView {
  view: View;
  back: View[];
  selected: string | null;
  overview: Overview | null;
}

const PREFS_KEY = "kitsu.prefs";
const LAST_KEY = "kitsu.project";
/** The switcher's counts: re-read this often while the window is visible. */
const PROJECTS_POLL_MS = 5000;

function loadPrefs(): Prefs {
  const d: Prefs = { vim: true, agent: "claude", policy: "ask", showDone: false, lang: "system", theme: "system", layout: "adaptive", tree: true, strip: true, paws: true, planDimDone: true };
  try {
    return { ...d, ...JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") };
  } catch {
    return d;
  }
}

function lastProject(): string | null {
  try {
    return localStorage.getItem(LAST_KEY);
  } catch {
    return null;
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

  /** The project on screen, by its id in the workspaces list. */
  repo = $state<string | null>(null);
  /** Every project with its counts, in list order. */
  projects = $state<ProjectSummary[]>([]);
  /** The switcher list under the rail header. */
  switcher = $state(false);
  /** Settings should put the cursor in "add a folder". */
  addingProject = $state(false);
  started = $state(false);

  private memory = new ViewMemory<RepoView>();
  private switchHooks: ((repo: string) => void)[] = [];
  private apis = new Map<string, RepoApi>();
  private refreshing = false;
  private again = false;
  private toastTimer: ReturnType<typeof setTimeout> | undefined;
  private projectsTimer: ReturnType<typeof setTimeout> | undefined;

  /** The open project's commands. Components take this once when they mount,
   *  and they're remounted on a switch, so each keeps talking to its own. */
  get api(): RepoApi {
    return this.apiFor(this.repo ?? "");
  }

  apiFor(repo: string): RepoApi {
    let a = this.apis.get(repo);
    if (!a) this.apis.set(repo, (a = repoApi(repo)));
    return a;
  }

  /** Per-project state that lives outside this class (editor buffers). */
  onSwitch(f: (repo: string) => void) {
    this.switchHooks.push(f);
  }

  /** The open project's entry in the list. */
  get project(): ProjectSummary | undefined {
    return this.projects.find((p) => p.id === this.repo);
  }

  async start() {
    this.mode = this.prefs.layout === "code" ? "code" : "work";
    this.applyTheme();
    darkQuery?.addEventListener("change", () => this.applyTheme());
    await i18n.use(this.prefs.lang === "system" ? systemLocale() : this.prefs.lang);
    try {
      const launch = await projects.launch();
      await this.refreshProjects();
      const first = pickStart(this.projects, launch?.id ?? null, lastProject());
      if (first) await this.switchTo(first);
    } catch (e) {
      this.loadError = errorText(e);
    }
    this.started = true;
    await onChanged((repo) => this.changed(repo));
    this.pollProjects();
  }

  private pollProjects() {
    clearTimeout(this.projectsTimer);
    this.projectsTimer = setTimeout(async () => {
      if (typeof document === "undefined" || document.visibilityState !== "hidden") await this.refreshProjects();
      this.pollProjects();
    }, PROJECTS_POLL_MS);
  }

  async refreshProjects() {
    try {
      this.projects = await projects.overview();
    } catch (e) {
      this.notify(errorText(e), "bad");
    }
  }

  /**
   * Put another project on screen. What you had open here is filed away and
   * comes back when you return; the other project's pages start where you
   * left them.
   */
  async switchTo(id: string) {
    this.switcher = false;
    if (id === this.repo) return;
    if (this.repo) this.memory.leave(this.repo, { view: this.view, back: this.back, selected: this.selected, overview: this.overview });
    const saved = this.memory.enter(id);
    this.repo = id;
    this.view = saved?.view ?? { kind: "home" };
    this.back = saved?.back ?? [];
    this.selected = saved?.selected ?? null;
    this.overview = saved?.overview ?? null;
    this.loadError = null;
    if (this.prefs.layout === "adaptive") this.mode = this.view.kind === "file" ? "code" : "work";
    try {
      localStorage.setItem(LAST_KEY, id);
    } catch {
      // Storage off: the next start opens the first project instead.
    }
    for (const f of this.switchHooks) f(id);
    await this.refresh();
  }

  /** ⌘O, the status bar, the rail header. The list hangs off the rail (or the
   *  file tree in the Code layout); with neither on screen, back to Work. */
  toggleSwitcher() {
    if (!this.switcher && this.mode === "code" && !this.prefs.tree) this.setMode("work");
    this.overlay = null;
    this.switcher = !this.switcher;
  }

  /** Add a folder to the list and open it. Throws the backend's refusal. */
  async addProject(path: string) {
    const p = await projects.add(path);
    await this.refreshProjects();
    await this.switchTo(p.id);
    const s = this.project;
    this.notify(s && !s.trusted ? t("projects.addedUntrusted", { name: p.name }) : t("projects.added", { name: p.name }), "ok");
    return p;
  }

  /** Take a project off the list. Nothing on disk changes. */
  async removeProject(id: string) {
    const before = this.projects;
    const p = await projects.remove(id);
    this.memory.forget(id);
    const next = afterRemoval(before, id, this.repo);
    await this.refreshProjects();
    if (next !== this.repo) {
      if (next) await this.switchTo(next);
      else {
        this.repo = null;
        this.overview = null;
        this.view = { kind: "home" };
      }
    }
    this.notify(t("projects.removed", { name: p.name }), "info");
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

  /** Coalesces bursts: at most one overview fetch in flight, one queued. An
   *  answer that arrives after a switch is about the other project and is
   *  dropped; the queued refresh reads the one on screen. */
  async refresh() {
    const repo = this.repo;
    if (!repo) return;
    if (this.refreshing) {
      this.again = true;
      return;
    }
    this.refreshing = true;
    try {
      const ov = await this.apiFor(repo).overview();
      if (repo !== this.repo) return;
      this.overview = ov;
      this.loadError = null;
      if (!this.selected && ov.tasks.length) this.selected = ov.tasks[0]!.id;
    } catch (e) {
      if (repo === this.repo) this.loadError = errorText(e);
    } finally {
      this.refreshing = false;
      if (this.again || repo !== this.repo) {
        this.again = false;
        void this.refresh();
      }
    }
  }

  /** Something moved in `repo` (null: maybe anywhere). */
  changed(repo: string | null = null) {
    if (repo === null || repo === this.repo) {
      this.tick++;
      void this.refresh();
    }
    void this.refreshProjects();
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
