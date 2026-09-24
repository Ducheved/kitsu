<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { api, errorText } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { statusText } from "../lib/status";
  import Glyph from "./Glyph.svelte";

  const ov = $derived(app.overview);
  const needs = $derived((ov?.tasks ?? []).filter((x) => x.attention === "needs_you"));
  const working = $derived((ov?.tasks ?? []).filter((x) => x.attention === "working"));
  const ready = $derived((ov?.tasks ?? []).filter((x) => x.attention === "ready"));

  async function trust() {
    try {
      await api.trustRepo();
      await app.refresh();
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }

  async function init() {
    try {
      await api.initRepo();
      await app.refresh();
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }
</script>

<div class="home">
  {#if app.loadError}
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

    <h1 class="calm">
      {#if needs.length}{t("home.needs", { n: needs.length })}{:else if working.length}{t("home.nothingNow")}{:else}{t("home.quiet")}{/if}
    </h1>
    <p class="sub">
      {#if working.length}{t("home.agentsWorking", { n: working.length })}{/if}
      {#if ready.length}{t("home.tasksReady", { n: ready.length })}{/if}
    </p>

    <div class="list">
      {#each [...needs, ...working, ...ready.slice(0, 3)] as task (task.id)}
        <button class="row" onclick={() => app.openTask(task.id)}>
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
    max-width: 720px;
    margin: 0 auto;
    padding: 12vh 40px 60px;
  }
  h1 {
    margin: 0 0 10px;
    font-size: 22px;
    font-weight: 650;
  }
  .calm {
    font-size: 28px;
    letter-spacing: -0.015em;
  }
  .sub {
    margin: 0 0 28px;
    color: var(--muted);
  }
  .card {
    margin-bottom: 14px;
  }
  .trust {
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
    border-color: var(--warn);
  }
  .problem {
    border-color: var(--bad);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border-radius: 10px;
    text-align: left;
  }
  .row:hover {
    background: var(--hover);
  }
  .title {
    font-weight: 500;
  }
  .row .hint {
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .keys {
    margin-top: 40px;
    display: flex;
    gap: 4px;
    align-items: center;
    flex-wrap: wrap;
  }
</style>
