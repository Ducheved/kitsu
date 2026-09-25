<script lang="ts">
  // Branches and worktrees of the open project, each marked as yours or as
  // a Kitsu run's (with its task). Read-only: nothing here changes git.
  import { app } from "../lib/app.svelte";
  import { errorText } from "../lib/api";
  import { i18n, t } from "../lib/i18n/index.svelte";
  import { shortPath, tildify } from "../lib/repos";
  import type { BranchRow, RepoTree, WorktreeRow } from "../lib/types";
  import Fox from "./Fox.svelte";
  // This project's commands, fixed for as long as the component lives.
  const api = app.api;

  let tree = $state<RepoTree | null>(null);
  let error = $state<string | null>(null);
  let showRuns = $state(false);

  $effect(() => {
    void app.tick;
    api
      .tree()
      .then((v) => {
        tree = v;
        error = null;
      })
      .catch((e) => (error = errorText(e)));
  });

  const root = $derived(app.overview?.repo.root ?? app.project?.root ?? "");
  const mine = $derived((tree?.branches ?? []).filter((b) => !b.run));
  const runs = $derived((tree?.branches ?? []).filter((b) => b.run));
  const taskTitle = (id: string | null) => (id ? (app.task(id)?.title ?? id) : null);

  function track(b: BranchRow): { text: string; tone: string } {
    if (!b.upstream) return { text: t("tree.noUpstream"), tone: "dim" };
    if (b.gone) return { text: t("tree.gone", { upstream: b.upstream }), tone: "warn" };
    if (!b.ahead && !b.behind) return { text: t("tree.inSync", { upstream: b.upstream }), tone: "ok" };
    return { text: t("tree.track", { ahead: b.ahead ?? 0, behind: b.behind ?? 0, upstream: b.upstream }), tone: b.behind ? "warn" : "work" };
  }

  function owner(w: WorktreeRow): { label: string; tone: string } {
    switch (w.owner.kind) {
      case "main":
        return { label: t("tree.main"), tone: "ok" };
      case "run":
        return { label: t("tree.run", { run: w.owner.run }), tone: "work" };
      case "integration":
        return { label: t("tree.integration"), tone: "work" };
      default:
        return { label: t("tree.yours"), tone: "" };
    }
  }

  const when = (secs: number) => (secs ? i18n.ago(secs * 1000) : "");
</script>

<article class="page">
  <header>
    <h1>{t("tree.title")}</h1>
    <p class="lede"><span class="proj">{app.project?.name ?? app.overview?.repo.name ?? ""}</span> <span class="mono dim">{tildify(root)}</span></p>
  </header>
  {#if error}<p class="tone-bad">{error}</p>{/if}

  {#if tree}
    <div class="section-title">{t("tree.worktrees")}<span class="count">{tree.worktrees.length}</span></div>
    {#each tree.worktrees as w (w.path)}
      {@const o = owner(w)}
      {@const task = w.owner.kind === "run" ? w.owner.task : null}
      <div class="row">
        <span class="tag {o.tone}">{o.label}</span>
        <span class="main">
          <span class="path mono">{shortPath(w.path, root)}</span>
          <span class="sub">
            {#if w.branch}<span class="mono">⎇ {w.branch}</span>{:else if w.head}<span class="mono">{t("tree.detached", { head: w.head.slice(0, 7) })}</span>{/if}
            {#if task}
              <button class="link" onclick={() => app.openTask(task)}>{taskTitle(task)}</button>
            {/if}
            {#if w.locked}<span class="tone-warn">{t("tree.locked")}</span>{/if}
            {#if w.prunable}<span class="tone-bad">{t("tree.prunable")}</span>{/if}
          </span>
        </span>
      </div>
    {/each}

    <div class="section-title">{t("tree.branches")}<span class="count">{mine.length}</span></div>
    {#each mine as b (b.name)}
      {@const tr = track(b)}
      <div class="row" class:current={b.current}>
        <span class="mark" aria-label={b.current ? t("tree.current") : undefined}>{b.current ? "●" : ""}</span>
        <span class="main">
          <span class="bname mono">{b.name}{#if b.current}<span class="cur">{t("tree.current")}</span>{/if}</span>
          <span class="sub"><span class="subject">{b.subject}</span><span class="dim">{when(b.date)}</span></span>
        </span>
        <span class="track tone-{tr.tone}">{tr.text}</span>
      </div>
    {/each}

    {#if runs.length}
      <div class="section-title runs-title">
        <span>{t("tree.runBranches")}<span class="count">{runs.length}</span></span>
        <button class="btn quiet mini" onclick={() => (showRuns = !showRuns)}>{showRuns ? t("tree.hideRuns") : t("tree.showRuns", { n: runs.length })}</button>
      </div>
      {#if showRuns}
        {#each runs as b (b.name)}
          <div class="row">
            <span class="mark"></span>
            <span class="main">
              <span class="bname mono">{b.name}</span>
              <span class="sub">
                {#if b.task}<button class="link" onclick={() => b.task && app.openTask(b.task)}>{taskTitle(b.task)}</button>{/if}
                <span class="dim">{when(b.date)}</span>
              </span>
            </span>
            <span class="track mono dim">{b.head}</span>
          </div>
        {/each}
      {/if}
    {/if}

    <p class="note hint"><Fox state="idle" size={22} /><span>{t("tree.readOnly")}</span></p>
  {/if}
</article>

<style>
  .page {
    max-width: 840px;
    margin: 0 auto;
    padding: var(--s7) var(--s7) var(--s8);
  }
  h1 {
    margin: 0 0 var(--s2);
    font-size: 26px;
    font-weight: 650;
  }
  .lede {
    display: flex;
    align-items: baseline;
    gap: var(--s2);
    margin: 0;
    color: var(--muted);
  }
  .proj {
    font-weight: 600;
    color: var(--text);
  }
  .dim {
    color: var(--faint);
  }
  .count {
    margin-left: 6px;
    font-weight: 500;
  }
  .runs-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .mini {
    height: 24px;
    padding: 0 10px;
    font-size: 12px;
    text-transform: none;
    letter-spacing: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s3);
    padding: var(--s3) 0;
    border-bottom: 1px solid var(--line);
  }
  .tag {
    flex: none;
    width: 112px;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }
  .tag.ok {
    color: var(--ok);
  }
  .tag.work {
    color: var(--work);
  }
  .mark {
    flex: none;
    width: 14px;
    color: var(--accent);
    font-size: 10px;
    text-align: center;
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .path,
  .bname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }
  .current .bname {
    font-weight: 600;
  }
  .cur {
    margin-left: var(--s2);
    padding: 1px 7px;
    border-radius: 9px;
    background: var(--accent-soft);
    color: var(--accent);
    font-family: var(--font);
    font-size: 11px;
    font-weight: 600;
  }
  .sub {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--s1) var(--s3);
    color: var(--muted);
    font-size: 12.5px;
  }
  .sub .mono {
    font-size: 11.5px;
  }
  .subject {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 48ch;
  }
  .track {
    flex: none;
    font-size: 12px;
    text-align: right;
  }
  .link {
    color: var(--text);
    text-decoration: underline;
    text-decoration-color: var(--line);
    text-underline-offset: 3px;
    transition:
      color var(--fast) var(--ease),
      text-decoration-color var(--fast) var(--ease);
  }
  .link:hover {
    color: var(--accent);
    text-decoration-color: var(--accent);
  }
  .note {
    display: flex;
    align-items: center;
    gap: var(--s3);
    margin-top: var(--s6);
    max-width: 68ch;
    line-height: 1.6;
  }
</style>
