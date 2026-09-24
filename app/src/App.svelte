<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "./lib/app.svelte";
  import { api, errorText } from "./lib/api";
  import type { Command } from "./lib/types";
  import Composer from "./components/Composer.svelte";
  import Help from "./components/Help.svelte";
  import Home from "./components/Home.svelte";
  import Palette from "./components/Palette.svelte";
  import Rail from "./components/Rail.svelte";
  import RulesPage from "./components/RulesPage.svelte";
  import TaskPage from "./components/TaskPage.svelte";

  let filter = $state("");
  let filtering = $state(false);
  let taskPage: TaskPage | undefined = $state();
  let rulesPage: RulesPage | undefined = $state();
  // The editor and diff views pull in CodeMirror (most of the bundle).
  // Load them on first use so startup only pays for the task UI.
  const editorPage = () => import("./components/EditorPage.svelte");
  const diffPage = () => import("./components/DiffPage.svelte");
  let pendingG = false;
  let gTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    void app.start();
  });

  const commands = $derived.by((): Command[] => {
    const c: Command[] = [
      { id: "new", title: "New task", hint: "n", group: "Commands", run: () => (app.overlay = "new") },
      { id: "rules", title: "Rules and checks", hint: "g r", group: "Commands", run: () => app.go({ kind: "rules" }) },
      { id: "home", title: "Home", hint: "g h", group: "Commands", run: () => app.go({ kind: "home" }) },
      { id: "open", title: "Open file…", hint: "⌘P", group: "Commands", run: () => setTimeout(() => (app.overlay = "files")) },
      { id: "checks", title: "Run all checks on my checkout", group: "Commands", run: () => runAllChecks() },
      { id: "vim", title: app.prefs.vim ? "Turn vim keys off" : "Turn vim keys on", group: "Commands", run: () => ((app.prefs.vim = !app.prefs.vim), app.savePrefs()) },
      { id: "done", title: app.prefs.showDone ? "Hide done tasks" : "Show done tasks", group: "Commands", run: () => ((app.prefs.showDone = !app.prefs.showDone), app.savePrefs()) },
      { id: "keys", title: "Keyboard shortcuts", hint: "?", group: "Commands", run: () => setTimeout(() => (app.overlay = "help")) },
    ];
    if (app.overview && !app.overview.repo.trusted) c.unshift({ id: "trust", title: "Trust this repository", group: "Commands", run: () => void api.trustRepo().then(() => app.refresh()) });
    for (const a of app.overview?.agents ?? []) {
      if (a.name !== app.prefs.agent) c.push({ id: `agent:${a.name}`, title: `Use ${a.name} for new runs`, hint: a.command, group: "Commands", run: () => ((app.prefs.agent = a.name), app.savePrefs()) });
    }
    return c;
  });

  async function runAllChecks() {
    try {
      const out = await api.runChecks();
      const bad = out.filter((e) => e.outcome !== "pass");
      app.notify(bad.length ? `Failing: ${bad.map((b) => b.check_name).join(", ")}` : `All ${out.length} checks pass.`, bad.length ? "bad" : "ok");
      app.changed();
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }

  function move(delta: number) {
    const list = app.visibleTasks(filter);
    if (!list.length) return;
    const i = Math.max(0, list.findIndex((t) => t.id === app.selected));
    const next = list[Math.min(list.length - 1, Math.max(0, i + delta))]!;
    app.selected = next.id;
    document.querySelector(`[data-task="${CSS.escape(next.id)}"]`)?.scrollIntoView({ block: "nearest" });
    // Following the selection feels right when a task is already open.
    if (app.view.kind === "task") app.openTask(next.id);
  }

  function inEditable(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
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
          const t = app.task((app.view as { id: string }).id);
          if (t?.status.kind === "asking") {
            const run = t.status.run;
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

<div class="app">
  <Rail bind:filter bind:filtering />
  <main class="scroll" class:flush={app.view.kind === "file" || app.view.kind === "diff"}>
    {#if app.view.kind === "task"}
      {#key app.view.id}
        <TaskPage bind:this={taskPage} id={app.view.id} />
      {/key}
    {:else if app.view.kind === "rules"}
      <RulesPage bind:this={rulesPage} />
    {:else if app.view.kind === "file"}
      {#await editorPage() then m}<m.default path={app.view.path} />{/await}
    {:else if app.view.kind === "diff"}
      {#await diffPage() then m}<m.default run={app.view.run} path={app.view.path} />{/await}
    {:else}
      <Home />
    {/if}
  </main>
</div>

{#if app.overlay === "palette"}
  <Palette {commands} />
{:else if app.overlay === "files"}
  <Palette commands={[]} files />
{:else if app.overlay === "new"}
  <Composer />
{:else if app.overlay === "help"}
  <Help />
{/if}

{#if app.toast}
  <div class="toast {app.toast.tone}" role="status">{app.toast.text}</div>
{/if}

<style>
  .app {
    display: grid;
    grid-template-columns: 300px 1fr;
    height: 100vh;
  }
  main {
    min-width: 0;
    min-height: 0;
    overflow: auto;
  }
  main.flush {
    overflow: hidden;
  }
  .toast {
    position: fixed;
    bottom: 22px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 30;
    max-width: min(640px, 90vw);
    padding: 10px 16px;
    border-radius: 12px;
    background: var(--text);
    color: var(--bg);
    box-shadow: var(--shadow);
    font-size: 13.5px;
    animation: rise 0.18s ease-out;
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
  @media (max-width: 980px) {
    .app {
      grid-template-columns: 260px 1fr;
    }
  }
</style>
