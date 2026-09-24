<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { api } from "../lib/api";
  import type { Attention, TaskView } from "../lib/types";
  import { t } from "../lib/i18n/index.svelte";
  import { inline } from "../lib/md";
  import { digestText, statusText } from "../lib/status";
  import Glyph from "./Glyph.svelte";

  let { filter = $bindable(""), filtering = $bindable(false) }: { filter?: string; filtering?: boolean } = $props();

  const groups: { key: Attention; title: () => string }[] = [
    { key: "needs_you", title: () => t("rail.needsYou") },
    { key: "working", title: () => t("rail.working") },
    { key: "ready", title: () => t("rail.ready") },
    { key: "waiting", title: () => t("rail.waiting") },
    { key: "quiet", title: () => t("rail.done") },
  ];

  const ov = $derived(app.overview);
  const tasks = $derived(app.visibleTasks(filter));
  const byGroup = $derived(
    groups.map((g) => ({ ...g, items: tasks.filter((x: TaskView) => x.attention === g.key) })).filter((g) => g.items.length),
  );
  const doneCount = $derived((ov?.tasks ?? []).filter((x) => x.attention === "quiet").length);
  const since = $derived(ov?.since);
  const showSince = $derived(!!since && since.items.length > 0 && since.from_seq > 0);

  let filterInput: HTMLInputElement | undefined = $state();
  $effect(() => {
    if (filtering) filterInput?.focus();
  });

  function dismissSince() {
    if (since) void api.markSeen(since.to_seq).then(() => app.refresh());
  }

  // First launch: nothing seen yet. Don't greet with a wall of history.
  $effect(() => {
    if (since && since.from_seq === 0 && since.to_seq > 0) void api.markSeen(since.to_seq);
  });
</script>

<aside class="rail">
  <header>
    <div class="repo">
      <span class="name">{ov?.repo.name ?? "Kitsu"}</span>
      {#if ov?.repo.branch}<span class="branch mono">{ov.repo.branch}</span>{/if}
    </div>
    {#if app.preview}<span class="pill dim" title={t("app.previewHint")}>{t("app.preview")}</span>{/if}
  </header>

  {#if filtering}
    <div class="filter">
      <input
        bind:this={filterInput}
        class="field"
        placeholder={t("rail.filter")}
        bind:value={filter}
        onkeydown={(e) => {
          if (e.key === "Escape" || e.key === "Enter") {
            if (e.key === "Escape") filter = "";
            filtering = false;
            (e.currentTarget as HTMLInputElement).blur();
          }
        }}
      />
    </div>
  {/if}

  <nav class="list scroll" aria-label="Tasks">
    {#if showSince && since}
      <section class="since">
        <div class="since-head">
          <span>{t("since.title")}</span>
          <button class="btn quiet mini" onclick={dismissSince}>{t("since.gotIt")}</button>
        </div>
        {#each since.items.slice(0, 5) as item, i (i)}
          {@const task = item.task ? app.task(item.task) : undefined}
          <button class="since-item" onclick={() => item.task && app.openTask(item.task)}>
            {#if task}<span class="since-task">{task.title}</span>{/if}
            <span class="since-text">{@html inline(digestText(item))}</span>
          </button>
        {/each}
      </section>
    {/if}

    {#each byGroup as g (g.key)}
      <section>
        <h2>{g.title()}<span class="count">{g.items.length}</span></h2>
        {#each g.items as task (task.id)}
          <button
            class="row"
            class:selected={app.selected === task.id}
            class:open={app.view.kind === "task" && app.view.id === task.id}
            data-task={task.id}
            onclick={() => app.openTask(task.id)}
          >
            <Glyph attention={task.attention} status={task.status} />
            <span class="text">
              <span class="title">{task.title}</span>
              <span class="reason">{statusText(task)}</span>
            </span>
          </button>
        {/each}
      </section>
    {/each}

    {#if !tasks.length && ov}
      <div class="empty">
        {#if filter}{t("rail.noMatch", { q: filter })}{:else}{@html t("rail.noTasks", { key: "<kbd>n</kbd>" })}{/if}
      </div>
    {/if}

    {#if doneCount && !app.prefs.showDone}
      <button
        class="show-done"
        onclick={() => {
          app.prefs.showDone = true;
          app.savePrefs();
        }}>{t("rail.showDone", { n: doneCount })}</button
      >
    {/if}
  </nav>

  <footer>
    <button class="foot-link" class:on={app.view.kind === "rules"} onclick={() => app.go({ kind: "rules" })}>
      {t("rail.rules")}
      {#if ov}<span class="hint">{t("rail.checks", { n: ov.counts.checks })} · {t("rail.decisions", { n: ov.counts.decisions })}{ov.counts.memory ? ` · ${t("rail.memory", { n: ov.counts.memory })}` : ""}{ov.counts.open_questions ? ` · ${t("rail.openQuestions", { n: ov.counts.open_questions })}` : ""}</span>{/if}
    </button>
    <div class="keys hint"><kbd>⌘K</kbd> {t("rail.keyCommands")} <kbd>n</kbd> {t("rail.keyNew")} <kbd>?</kbd> {t("rail.keyKeys")}</div>
  </footer>
</aside>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--rail);
    border-right: 1px solid var(--line);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 18px 18px 10px;
    -webkit-app-region: drag;
  }
  .repo {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .name {
    font-weight: 650;
    font-size: 15px;
  }
  .branch {
    color: var(--muted);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .filter {
    padding: 0 12px 8px;
  }
  .list {
    flex: 1;
    min-height: 0;
    padding: 4px 8px 16px;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 16px 10px 4px;
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--faint);
  }
  .count {
    font-weight: 500;
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border-radius: 9px;
    text-align: left;
  }
  .row :global(.glyph) {
    margin-top: 3px;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.selected {
    background: var(--active);
  }
  .row.open {
    background: var(--elev);
    box-shadow: 0 0 0 1px var(--line);
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .reason {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
    font-size: 12.5px;
  }
  .since {
    margin: 6px 2px 4px;
    padding: 10px 12px 8px;
    border-radius: 12px;
    background: var(--elev);
    border: 1px solid var(--line);
  }
  .since-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
    margin-bottom: 4px;
  }
  .mini {
    height: 22px;
    padding: 0 8px;
    font-size: 12px;
  }
  .since-item {
    display: block;
    width: 100%;
    padding: 3px 0;
    text-align: left;
    font-size: 12.5px;
    color: var(--text);
  }
  .since-item:hover .since-task {
    color: var(--accent);
  }
  .since-task {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .since-text {
    color: var(--muted);
  }
  .empty {
    padding: 24px 12px;
    color: var(--muted);
    font-size: 13px;
  }
  .show-done {
    margin: 10px 10px 0;
    color: var(--faint);
    font-size: 12.5px;
  }
  .show-done:hover {
    color: var(--text);
  }
  footer {
    padding: 10px 12px 14px;
    border-top: 1px solid var(--line);
  }
  .foot-link {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    width: 100%;
    padding: 8px 10px;
    border-radius: 9px;
    font-weight: 500;
    text-align: left;
  }
  .foot-link:hover,
  .foot-link.on {
    background: var(--hover);
  }
  .keys {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 8px 10px 0;
  }
</style>
