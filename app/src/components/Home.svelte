<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { api, errorText } from "../lib/api";
  import Glyph from "./Glyph.svelte";

  const ov = $derived(app.overview);
  const needs = $derived((ov?.tasks ?? []).filter((t) => t.attention === "needs_you"));
  const working = $derived((ov?.tasks ?? []).filter((t) => t.attention === "working"));
  const ready = $derived((ov?.tasks ?? []).filter((t) => t.attention === "ready"));

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
      <h1>Can't open this folder</h1>
      <p class="tone-bad">{app.loadError}</p>
      <p class="hint">Start Kitsu from inside a git repository, or pass its path: <span class="mono">kitsu-app ~/code/project</span>.</p>
    </div>
  {:else if ov && !ov.repo.initialized}
    <div class="card">
      <h1>Set up {ov.repo.name}</h1>
      <p>Kitsu keeps tasks, rules and decisions as small files in <span class="mono">.kitsu/</span>, next to your code, versioned with it.</p>
      <button class="btn primary" onclick={init}>Create .kitsu/</button>
    </div>
  {:else if ov}
    {#if !ov.repo.trusted}
      <div class="card trust">
        <strong>Trust this repository?</strong>
        <span>Agents and checks run code from it (tests, build scripts). Until you say so, Kitsu won't run anything.</span>
        <button class="btn" onclick={trust}>Trust {ov.repo.name}</button>
      </div>
    {/if}
    {#each ov.problems as p (p.path)}
      <div class="card problem"><strong class="tone-bad">Can't read {p.path}.</strong> {p.detail}. Whatever rule it holds is not being enforced or shown to agents.</div>
    {/each}

    <h1 class="calm">
      {#if needs.length}{needs.length === 1 ? "One thing needs you." : `${needs.length} things need you.`}{:else if working.length}Nothing needs you right now.{:else}All quiet.{/if}
    </h1>
    <p class="sub">
      {#if working.length}{working.length} agent{working.length === 1 ? " is" : "s are"} working.{/if}
      {#if ready.length}{ready.length} task{ready.length === 1 ? "" : "s"} ready to start.{/if}
    </p>

    <div class="list">
      {#each [...needs, ...working, ...ready.slice(0, 3)] as t (t.id)}
        <button class="row" onclick={() => app.openTask(t.id)}>
          <Glyph attention={t.attention} status={t.status} />
          <span class="t">{t.title}</span>
          <span class="hint">{t.reason}</span>
        </button>
      {/each}
    </div>

    <p class="keys hint"><kbd>j</kbd><kbd>k</kbd> to move, <kbd>⏎</kbd> to open, <kbd>n</kbd> for a new task, <kbd>⌘K</kbd> for everything else.</p>
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
  .t {
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
