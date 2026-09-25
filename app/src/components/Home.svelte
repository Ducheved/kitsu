<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { errorText } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { statusText } from "../lib/status";
  import Fox, { type FoxState } from "./Fox.svelte";
  import Glyph from "./Glyph.svelte";
  import AddProject from "./AddProject.svelte";
  // This project's commands, fixed for as long as the component lives.
  const api = app.api;

  const ov = $derived(app.overview);
  const needs = $derived((ov?.tasks ?? []).filter((x) => x.attention === "needs_you"));
  const working = $derived((ov?.tasks ?? []).filter((x) => x.attention === "working"));
  const ready = $derived((ov?.tasks ?? []).filter((x) => x.attention === "ready"));
  const fox = $derived<FoxState>(needs.length ? "asking" : working.length ? "working" : ready.length ? "idle" : "sleeping");

  async function trust() {
    try {
      await api.trustRepo();
      app.changed(api.repo);
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }

  async function init() {
    try {
      await api.initRepo();
      app.changed(api.repo);
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }
</script>

<div class="home">
  {#if app.started && !app.repo && !app.loadError}
    <div class="card welcome">
      <Fox state="idle" size={48} />
      <h1>{t("home.noProjects")}</h1>
      <p>{@html t("home.noProjectsBody", { key: "<kbd>⌘O</kbd>" })}</p>
      <AddProject focus />
    </div>
  {:else if app.loadError}
    <div class="card">
      <h1>{t("home.cantOpen")}</h1>
      <p class="tone-bad">{app.loadError}</p>
      <p class="hint">{t("home.cantOpenHint")} <span class="mono">kitsu-app ~/code/project</span></p>
    </div>
  {:else if ov && !ov.repo.initialized}
    <div class="card">
      <h1>{t("home.setup", { name: ov.repo.name })}</h1>
      <p>{t("home.setupBody")}</p>
      <button class="btn primary" onclick={init}>{t("home.create")}</button>
    </div>
  {:else if ov}
    {#if !ov.repo.trusted}
      <div class="card trust">
        <strong>{t("home.trustTitle")}</strong>
        <span>{t("home.trustBody")}</span>
        <button class="btn" onclick={trust}>{t("home.trust", { name: ov.repo.name })}</button>
      </div>
    {/if}
    {#each ov.problems as p (p.path)}
      <div class="card problem tone-bad">{t("home.problem", { path: p.path, detail: p.detail })}</div>
    {/each}

    <header class="greeting">
      <Fox state={fox} size={56} />
      <div>
        <h1 class="calm">
          {#if needs.length}{t("home.needs", { n: needs.length })}{:else if working.length}{t("home.nothingNow")}{:else}{t("home.quiet")}{/if}
        </h1>
        <p class="sub">
          {#if working.length}{t("home.agentsWorking", { n: working.length })}{/if}
          {#if ready.length}{t("home.tasksReady", { n: ready.length })}{/if}
        </p>
      </div>
    </header>

    <div class="list" data-tour="home-needs">
      {#each [...needs, ...working, ...ready.slice(0, 3)] as task, i (task.id)}
        <button class="row press enter" style:--i={i} onclick={() => app.openTask(task.id)}>
          <Glyph attention={task.attention} status={task.status} />
          <span class="title">{task.title}</span>
          <span class="hint">{statusText(task)}</span>
        </button>
      {/each}
    </div>

    <p class="keys hint">
      <kbd>j</kbd><kbd>k</kbd> {t("home.keyMove")} <kbd>⏎</kbd> {t("home.keyOpen")} <kbd>n</kbd> {t("home.keyNew")} <kbd>⌘K</kbd> {t("home.keyAll")}
    </p>
  {/if}
</div>

<style>
  .home {
    max-width: 760px;
    margin: 0 auto;
    padding: 14vh var(--s7) var(--s8);
  }
  h1 {
    margin: 0 0 var(--s3);
    font-size: 22px;
    font-weight: 650;
  }
  .greeting {
    display: flex;
    align-items: center;
    gap: var(--s5);
    margin-bottom: var(--s6);
  }
  .calm {
    margin: 0 0 var(--s1);
    font-size: 28px;
    line-height: 1.25;
    letter-spacing: -0.015em;
  }
  .sub {
    margin: 0;
    color: var(--muted);
  }
  .card {
    margin-bottom: var(--s4);
  }
  .welcome {
    display: flex;
    flex-direction: column;
    gap: var(--s2);
  }
  .welcome h1 {
    margin: var(--s2) 0 0;
  }
  .welcome p {
    margin: 0 0 var(--s2);
    color: var(--muted);
  }
  .trust {
    display: flex;
    flex-direction: column;
    gap: var(--s2);
    align-items: flex-start;
    border-color: var(--warn);
  }
  .problem {
    border-color: var(--bad);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--s1);
    /* Rows keep their own padding; pull them out so text lines up with the heading. */
    margin: 0 calc(-1 * var(--s4));
  }
  .row {
    --press: 0.99;
    display: flex;
    align-items: center;
    gap: var(--s3);
    padding: var(--s3) var(--s4);
    border-radius: 12px;
    text-align: left;
  }
  .row:hover {
    background: var(--hover);
  }
  .title {
    font-weight: 500;
  }
  .row .hint {
    color: var(--muted);
    margin-left: auto;
    padding-left: var(--s4);
    flex: none;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .keys {
    margin-top: var(--s7);
    display: flex;
    gap: var(--s1);
    align-items: center;
    flex-wrap: wrap;
  }
</style>
