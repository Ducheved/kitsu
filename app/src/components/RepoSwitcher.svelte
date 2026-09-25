<script lang="ts">
  // The rail's header: the open project and its branch. Click it (or ⌘O) for
  // a compact list of every project with what needs you there; 1–9 picks.
  import { untrack } from "svelte";
  import { app } from "../lib/app.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import { needsElsewhere, projectAtKey, step, tildify } from "../lib/repos";

  const list = $derived(app.projects);
  const current = $derived(app.project);
  const name = $derived(current?.name ?? app.overview?.repo.name ?? "Kitsu");
  const branch = $derived(app.overview?.repo.branch ?? current?.branch ?? null);
  const elsewhere = $derived(needsElsewhere(list, app.repo));

  let { compact = false }: { compact?: boolean } = $props();

  let index = $state(0);
  let panel: HTMLDivElement | undefined = $state();
  let button: HTMLButtonElement | undefined = $state();
  // Fixed, under the button: the rail and the file tree both clip overflow.
  let at = $state({ top: 0, left: 0 });

  $effect(() => {
    if (!app.switcher) return;
    // Only opening it counts; the refreshed list mustn't re-run this.
    untrack(() => {
      index = Math.max(0, list.findIndex((p) => p.id === app.repo));
      const r = button?.getBoundingClientRect();
      if (r) at = { top: r.bottom + 6, left: Math.max(8, r.left - 4) };
      void app.refreshProjects();
      queueMicrotask(() => panel?.focus());
    });
  });

  function close() {
    app.switcher = false;
  }

  function toTree() {
    close();
    app.go({ kind: "tree" });
  }

  function toAdd() {
    close();
    app.addingProject = true;
    app.overlay = "settings";
  }

  function key(e: KeyboardEvent) {
    const n = list.length;
    let handled = true;
    if (e.key === "ArrowDown" || e.key === "j") index = step(index, 1, n);
    else if (e.key === "ArrowUp" || e.key === "k") index = step(index, -1, n);
    else if (e.key === "Enter" || e.key === "o") {
      const p = list[index];
      if (p) void app.switchTo(p.id);
    } else if (e.key === "Escape") close();
    else if (e.key === "b") toTree();
    else if (e.key === "a") toAdd();
    else {
      const id = projectAtKey(list, e.key);
      if (id) void app.switchTo(id);
      else handled = false;
    }
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  }
</script>

<div class="switcher">
  <button
    bind:this={button}
    class="current press"
    class:compact
    class:open={app.switcher}
    aria-haspopup="listbox"
    aria-expanded={app.switcher}
    title="{t('project.switch')} · ⌘O"
    data-tour="switcher"
    onclick={() => (app.switcher = !app.switcher)}
  >
    <span class="name">{name}</span>
    <svg class="chev" width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M2 3.5 L5 6.5 L8 3.5" /></svg>
    {#if branch && !compact}<span class="branch mono">{branch}</span>{/if}
    {#if elsewhere}<span class="elsewhere" title={t("project.elsewhere", { n: elsewhere })}>{elsewhere}</span>{/if}
  </button>

  {#if app.switcher}
    <div class="scrim" role="presentation" onclick={close}></div>
    <div class="panel" style:top="{at.top}px" style:left="{at.left}px" role="dialog" aria-label={t("project.switch")} tabindex="-1" bind:this={panel} onkeydown={key}>
      <div class="rows" role="listbox" aria-label={t("help.projects")}>
        {#each list as p, i (p.id)}
          <button
            class="row"
            class:on={i === index}
            class:here={p.id === app.repo}
            role="option"
            aria-selected={p.id === app.repo}
            onmouseenter={() => (index = i)}
            onclick={() => void app.switchTo(p.id)}
          >
            <span class="digit">{i < 9 ? i + 1 : ""}</span>
            <span class="text">
              <span class="line">
                <span class="pname">{p.name}</span>
                <span class="pbranch mono">{p.branch ?? t("project.detached")}</span>
              </span>
              <span class="root mono">{tildify(p.root)}</span>
            </span>
            <span class="facts">
              {#if p.error}
                <span class="tone-bad" title={p.error}>{p.missing ? t("project.gone") : t("project.unreadable")}</span>
              {:else}
                {#if p.needs_you}<span class="needs"><span class="dot"></span>{p.needs_you}</span>{/if}
                {#if p.working}<span class="working" title={t("project.working", { n: p.working })}>◌ {p.working}</span>{/if}
                {#if !p.trusted}<span class="tone-warn small">{t("sb.untrusted")}</span>{/if}
              {/if}
            </span>
          </button>
        {/each}
      </div>
      <div class="actions">
        {#if app.repo}
          <button class="action" onclick={toTree}>{t("project.tree")}<kbd>b</kbd></button>
        {/if}
        <button class="action" onclick={toAdd}>{t("project.add")}<kbd>a</kbd></button>
      </div>
      <div class="keys hint">{t("project.keys")}</div>
    </div>
  {/if}
</div>

<style>
  .switcher {
    position: relative;
    min-width: 0;
    flex: 1;
  }
  .current {
    --press: 0.985;
    display: flex;
    align-items: baseline;
    gap: 6px;
    max-width: 100%;
    margin: -4px -8px;
    padding: 4px 8px;
    border-radius: 8px;
    text-align: left;
    -webkit-app-region: no-drag;
  }
  .current:hover,
  .current.open {
    background: var(--hover);
  }
  .name {
    flex: none;
    max-width: 60%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 650;
    font-size: 15px;
  }
  .compact .name {
    max-width: 150px;
    font-size: 12.5px;
    font-weight: 600;
  }
  .chev {
    flex: none;
    align-self: center;
    fill: none;
    stroke: var(--faint);
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform var(--fast) var(--ease);
  }
  .open .chev {
    transform: rotate(180deg);
  }
  .branch {
    min-width: 5ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
    font-size: 12px;
  }
  .elsewhere {
    flex: none;
    align-self: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--warn-soft);
    color: var(--warn);
    font-size: 11px;
    font-weight: 600;
    line-height: 18px;
    text-align: center;
  }
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 15;
  }
  .panel {
    position: fixed;
    z-index: 16;
    width: 296px;
    padding: var(--s2);
    border-radius: 12px;
    background: var(--elev);
    border: 1px solid var(--line);
    box-shadow: var(--shadow);
    outline: none;
    animation: pop-in var(--quick) var(--ease);
    -webkit-app-region: no-drag;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 50vh;
    overflow: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s2);
    width: 100%;
    padding: var(--s2) var(--s2);
    border-radius: 8px;
    text-align: left;
    transition: background-color var(--fast) var(--ease);
  }
  .row.on {
    background: var(--active);
  }
  .digit {
    flex: none;
    width: 14px;
    color: var(--faint);
    font-family: var(--mono);
    font-size: 11px;
    text-align: center;
  }
  .here .digit {
    color: var(--accent);
    font-weight: 700;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }
  .pname {
    flex: none;
    max-width: 65%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .here .pname {
    font-weight: 650;
  }
  .pbranch,
  .root {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
    font-size: 11.5px;
  }
  .root {
    color: var(--faint);
    font-size: 11px;
  }
  .facts {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--s2);
    font-size: 12px;
  }
  .needs {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--warn);
    font-weight: 600;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--warn);
  }
  .working {
    color: var(--work);
  }
  .small {
    font-size: 11px;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: var(--s2);
    padding-top: var(--s2);
    border-top: 1px solid var(--line);
  }
  .action {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px var(--s2) 6px calc(var(--s2) + 14px + var(--s2));
    border-radius: 8px;
    color: var(--muted);
    font-size: 13px;
    text-align: left;
    transition:
      background-color var(--fast) var(--ease),
      color var(--fast) var(--ease);
  }
  .action:hover {
    background: var(--hover);
    color: var(--text);
  }
  .keys {
    padding: var(--s2) var(--s2) 2px calc(var(--s2) + 14px + var(--s2));
    font-size: 11.5px;
  }
</style>
