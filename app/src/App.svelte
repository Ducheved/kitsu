<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "./lib/app.svelte";
  import { api, errorText } from "./lib/api";
  import { buffers } from "./lib/buffers.svelte";
  import { LOCALES, i18n, t } from "./lib/i18n/index.svelte";
  import type { Command } from "./lib/types";
  import AgentStrip from "./components/AgentStrip.svelte";
  import Composer from "./components/Composer.svelte";
  import Explorer from "./components/Explorer.svelte";
  import Help from "./components/Help.svelte";
  import Home from "./components/Home.svelte";
  import Palette from "./components/Palette.svelte";
  import Rail from "./components/Rail.svelte";
  import RulesPage from "./components/RulesPage.svelte";
  import Settings from "./components/Settings.svelte";
  import StatusBar from "./components/StatusBar.svelte";
  import TaskPage from "./components/TaskPage.svelte";
  import Fox from "./components/Fox.svelte";
  import Paws from "./components/Paws.svelte";
  import Tour from "./components/Tour.svelte";
  import { tour } from "./lib/tour.svelte";
  import { kitsuTour } from "./lib/tour-steps";

  let filter = $state("");
  let filtering = $state(false);
  let taskPage: TaskPage | undefined = $state();
  let rulesPage: RulesPage | undefined = $state();
  let explorer: Explorer | undefined = $state();
  // The editor and diff views pull in CodeMirror (most of the bundle).
  // Load them on first use so startup only pays for the task UI.
  const codeWorkspace = () => import("./components/CodeWorkspace.svelte");
  const diffPage = () => import("./components/DiffPage.svelte");
  let pendingG = false;
  let gTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    void app.start().then(() => {
      // Preview data is where the tour can't hurt anything: offer it once.
      if (app.preview && !app.loadError && tour.unseen()) setTimeout(() => tour.start(), 400);
    });
  });

  const code = $derived(app.mode === "code");
  // In the Code layout the editor is the page unless a task, the rules or a
  // diff is open; in the Work layout it's one page among the others.
  const showEditor = $derived(app.view.kind === "file" || (code && app.view.kind === "home" && buffers.open.length > 0));
  // Each page arrives with a short fade; the editor is one page however many
  // files it switches between, so it doesn't flicker on every tab.
  // Placed explicitly so the paw prints can share main's grid cell.
  const mainColumn = $derived(code && !app.prefs.tree ? "1" : "2");
  const viewKey = $derived(
    showEditor ? "editor" : app.view.kind === "task" ? `task:${app.view.id}` : app.view.kind === "diff" ? `diff:${app.view.run}:${app.view.path}` : app.view.kind,
  );

  const commands = $derived.by((): Command[] => {
    const cmd = (id: string, title: string, run: () => void, hint?: string): Command => ({ id, title, hint, group: "commands", run });
    const later = (f: () => void) => () => setTimeout(f);
    const c: Command[] = [
      cmd("new", t("cmd.new"), later(() => (app.overlay = "new")), "n"),
      cmd("rules", t("cmd.rules"), () => app.go({ kind: "rules" }), "g r"),
      cmd("home", t("cmd.home"), () => app.go({ kind: "home" }), "g h"),
      cmd("open", t("cmd.open"), later(() => (app.overlay = "files")), "⌘P"),
      cmd("mode", code ? t("cmd.work") : t("cmd.code"), () => app.setMode(code ? "work" : "code"), code ? "⌘1" : "⌘2"),
      cmd("checks", t("cmd.checks"), () => void runAllChecks()),
      cmd("vim", app.prefs.vim ? t("cmd.vimOff") : t("cmd.vimOn"), () => ((app.prefs.vim = !app.prefs.vim), app.savePrefs())),
      cmd("done", app.prefs.showDone ? t("cmd.hideDone") : t("cmd.showDone"), () => ((app.prefs.showDone = !app.prefs.showDone), app.savePrefs())),
      cmd("settings", t("cmd.settings"), later(() => (app.overlay = "settings")), "⌘,"),
      cmd("keys", t("cmd.keys"), later(() => (app.overlay = "help")), "?"),
      cmd("tour", t("cmd.tour"), later(() => tour.start())),
    ];
    for (const th of ["system", "light", "dark"] as const) {
      if (th !== app.prefs.theme) c.push(cmd(`theme:${th}`, t("cmd.theme", { theme: t(`theme.${th}`) }), () => app.setTheme(th)));
    }
    for (const [code, name] of Object.entries(LOCALES) as [keyof typeof LOCALES, string][]) {
      if (code !== i18n.locale) c.push(cmd(`lang:${code}`, t("cmd.language", { language: name }), () => void app.setLang(code)));
    }
    if (app.overview && !app.overview.repo.trusted) c.unshift(cmd("trust", t("cmd.trust"), () => void api.trustRepo().then(() => app.refresh())));
    for (const a of app.overview?.agents ?? []) {
      if (a.name !== app.prefs.agent) c.push(cmd(`agent:${a.name}`, t("cmd.useAgent", { agent: a.name }), () => ((app.prefs.agent = a.name), app.savePrefs()), a.command));
    }
    return c;
  });

  async function runAllChecks() {
    try {
      const out = await api.runChecks();
      const bad = out.filter((e) => e.outcome !== "pass");
      app.notify(bad.length ? t("cmd.failing", { names: i18n.list(bad.map((b) => b.check_name)) }) : t("rules.allPass", { n: out.length }), bad.length ? "bad" : "ok");
      app.changed();
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }

  function move(delta: number) {
    const list = app.visibleTasks(filter);
    if (!list.length) return;
    const i = Math.max(0, list.findIndex((x) => x.id === app.selected));
    const next = list[Math.min(list.length - 1, Math.max(0, i + delta))]!;
    app.selected = next.id;
    document.querySelector(`[data-task="${CSS.escape(next.id)}"]`)?.scrollIntoView({ block: "nearest" });
    // Following the selection feels right when a task is already open.
    if (app.view.kind === "task") app.openTask(next.id);
  }

  function inEditable(target: EventTarget | null): boolean {
    const el = target as HTMLElement | null;
    return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable || !!el.closest(".cm-editor"));
  }

  function onKey(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === "k") {
      e.preventDefault();
      app.overlay = app.overlay === "palette" ? null : "palette";
      return;
    }
    if (mod && e.key.toLowerCase() === "p") {
      e.preventDefault();
      app.overlay = "files";
      return;
    }
    if (mod && e.key === ",") {
      e.preventDefault();
      app.overlay = app.overlay === "settings" ? null : "settings";
      return;
    }
    if (mod && (e.key === "1" || e.key === "2")) {
      e.preventDefault();
      app.setMode(e.key === "1" ? "work" : "code");
      return;
    }
    if (e.ctrlKey && e.key === "Tab" && app.mode === "code") {
      e.preventDefault();
      buffers.cycle(e.shiftKey ? -1 : 1);
      if (app.view.kind !== "file" && buffers.active) app.go({ kind: "file", path: buffers.active });
      return;
    }
    if (mod && e.key.toLowerCase() === "b" && app.mode === "code") {
      e.preventDefault();
      app.prefs.tree = !app.prefs.tree;
      app.savePrefs();
      return;
    }
    if (mod && e.key.toLowerCase() === "j" && app.mode === "code") {
      e.preventDefault();
      app.prefs.strip = !app.prefs.strip;
      app.savePrefs();
      return;
    }
    if (app.overlay) return;
    if (inEditable(e.target)) return;
    if (mod || e.altKey) return;

    if (pendingG) {
      pendingG = false;
      clearTimeout(gTimer);
      if (e.key === "r") app.go({ kind: "rules" });
      else if (e.key === "h") app.go({ kind: "home" });
      e.preventDefault();
      return;
    }

    const onTask = app.view.kind === "task";
    switch (e.key) {
      case "e":
        if (code && app.prefs.tree) explorer?.focus();
        break;
      case "j":
      case "ArrowDown":
        move(1);
        break;
      case "k":
      case "ArrowUp":
        move(-1);
        break;
      case "Enter":
      case "o":
        if (app.selected) app.openTask(app.selected);
        break;
      case "/":
        filtering = true;
        break;
      case ":":
        app.overlay = "palette";
        break;
      case "?":
        app.overlay = "help";
        break;
      case "n":
        app.overlay = "new";
        break;
      case "g":
        pendingG = true;
        gTimer = setTimeout(() => (pendingG = false), 900);
        break;
      case "Escape":
        if (app.view.kind !== "home") app.goBack();
        break;
      case "r":
        if (onTask) void taskPage?.start();
        break;
      case "s":
        if (onTask) void taskPage?.stop();
        break;
      case "a":
        if (onTask) taskPage?.accept();
        break;
      case "c":
        if (onTask) taskPage?.continueWork();
        break;
      case "x":
        if (onTask) taskPage?.discard();
        break;
      default:
        if (/^[1-9]$/.test(e.key) && onTask) {
          const task = app.task((app.view as { id: string }).id);
          if (task?.status.kind === "asking") {
            const run = task.status.run;
            const ask = app.overview?.asks.find((a) => a.run === run);
            const opt = ask?.request.options[Number(e.key) - 1];
            if (ask && opt) void api.answerAsk(ask.id, opt.optionId).then(() => app.changed());
          }
        } else return;
    }
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onKey} />

{#snippet page()}
  {#if showEditor}
    {#await codeWorkspace() then m}<m.default path={app.view.kind === "file" ? app.view.path : undefined} />{/await}
  {:else if app.view.kind === "task"}
    {#key app.view.id}
      <TaskPage bind:this={taskPage} id={app.view.id} />
    {/key}
  {:else if app.view.kind === "rules"}
    <RulesPage bind:this={rulesPage} />
  {:else if app.view.kind === "diff"}
    {#await diffPage() then m}<m.default run={app.view.run} path={app.view.path} />{/await}
  {:else}
    <Home />
  {/if}
{/snippet}

<div class="app" class:code class:no-tree={code && !app.prefs.tree} class:no-strip={code && !app.prefs.strip}>
  {#if code}
    {#if app.prefs.tree}<Explorer bind:this={explorer} />{/if}
  {:else}
    <Rail bind:filter bind:filtering />
  {/if}
  <main class="scroll" class:flush={showEditor || app.view.kind === "diff"} style:grid-column={mainColumn}>
    {#key viewKey}
      <div class="view" class:fill={showEditor || app.view.kind === "diff"}>{@render page()}</div>
    {/key}
  </main>
  <!-- Same grid cell as main, laid over its bottom-right corner. Not over the
       editor or a diff, where code runs to the edge. They walk in once at
       start and again after each accept. -->
  {#if app.prefs.paws && !showEditor && app.view.kind !== "diff"}
    <div class="corner" style:grid-column={mainColumn} aria-hidden="true">
      {#key app.accepted}<span class="trail"><Paws count={5} heading={-38} size={11} walk delay={app.accepted ? 700 : 300} /></span>{/key}
    </div>
  {/if}
  {#if code && app.prefs.strip}<AgentStrip />{/if}
  <div class="status"><StatusBar /></div>
</div>

{#if app.overlay === "palette"}
  <Palette {commands} />
{:else if app.overlay === "files"}
  <Palette commands={[]} files />
{:else if app.overlay === "new"}
  <Composer />
{:else if app.overlay === "help"}
  <Help />
{:else if app.overlay === "settings"}
  <Settings />
{/if}

{#if tour.open}
  <Tour steps={kitsuTour} onclose={() => tour.stop()} paused={app.overlay !== null && app.overlay !== "palette"} />
{/if}

{#if app.toast}
  {#key app.toast}
    <div class="toast {app.toast.tone}" role="status">
      {#if app.toast.cheer}<Fox state="happy" size={28} />{/if}
      <span>{app.toast.text}</span>
      {#if app.toast.cheer && app.prefs.paws}<span class="leaving"><Paws count={3} heading={90} size={9} walk delay={450} /></span>{/if}
    </div>
  {/key}
{/if}

<style>
  .app {
    display: grid;
    grid-template-columns: 320px 1fr;
    grid-template-rows: 1fr auto;
    height: 100vh;
  }
  .app.code {
    grid-template-columns: 250px 1fr 280px;
  }
  .app.code.no-tree {
    grid-template-columns: 1fr 280px;
  }
  .app.code.no-strip {
    grid-template-columns: 250px 1fr;
  }
  .app.code.no-tree.no-strip {
    grid-template-columns: 1fr;
  }
  .status {
    grid-column: 1 / -1;
  }
  main {
    grid-row: 1;
    min-width: 0;
    min-height: 0;
    overflow: auto;
  }
  .corner {
    grid-row: 1;
    z-index: 1;
    align-self: end;
    display: flex;
    justify-content: flex-end;
    padding: 0 var(--s5) var(--s4) 0;
    pointer-events: none;
    color: var(--paw);
    container-type: inline-size;
  }
  /* Only where the reading column (760px at most) leaves a margin wider
     than the trail, so the prints never sit on text. */
  .trail {
    display: none;
  }
  @container (min-width: 900px) {
    .trail {
      display: block;
    }
  }
  .leaving {
    display: flex;
    margin-left: var(--s1);
    opacity: 0.55;
    --paw-walk: var(--accent);
  }
  main.flush {
    overflow: hidden;
  }
  .view {
    animation: view-in var(--quick) var(--ease);
  }
  .view.fill {
    height: 100%;
  }
  @keyframes view-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
  .toast {
    position: fixed;
    bottom: var(--s7);
    left: 50%;
    transform: translateX(-50%);
    z-index: 30;
    display: flex;
    align-items: center;
    gap: var(--s3);
    max-width: min(640px, 90vw);
    padding: var(--s3) var(--s4);
    border-radius: 12px;
    background: var(--text);
    color: var(--bg);
    box-shadow: var(--shadow);
    font-size: 13.5px;
    animation: rise var(--quick) var(--ease);
  }
  .toast.bad {
    background: var(--bad);
    color: #fff;
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translate(-50%, 6px);
    }
  }
  @media (max-width: 1200px) {
    .app {
      grid-template-columns: 280px 1fr;
    }
  }
  @media (max-width: 980px) {
    .app {
      grid-template-columns: 260px 1fr;
    }
    .app.code {
      grid-template-columns: 210px 1fr 240px;
    }
  }
</style>
