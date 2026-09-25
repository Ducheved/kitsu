<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { errorText } from "../lib/api";
  import { i18n, t } from "../lib/i18n/index.svelte";
  import { inline, render } from "../lib/md";
  import { estimateTokens, runWord, statusText, tokenParams } from "../lib/status";
  import type { Ask, Run, TaskDetail } from "../lib/types";
  import Fox from "./Fox.svelte";
  import Glyph from "./Glyph.svelte";
  import ReviewCard from "./ReviewCard.svelte";
  import RunActivity from "./RunActivity.svelte";
  // This project's commands, fixed for as long as the component lives.
  const api = app.api;

  let { id }: { id: string } = $props();

  let detail = $state<TaskDetail | null>(null);
  let error = $state<string | null>(null);
  let note = $state("");
  let starting = $state(false);
  let answer = $state<Record<string, string>>({});
  let showBrief = $state(false);
  let openRun = $state<string | null>(null);
  let reviewCard: ReviewCard | undefined = $state();

  $effect(() => {
    const taskId = id;
    void app.tick;
    api
      .task(taskId)
      .then((d) => {
        if (taskId === id) {
          detail = d;
          error = null;
        }
      })
      .catch((e) => (error = errorText(e)));
  });

  const view = $derived(app.task(id));
  const status = $derived(view?.status);
  const statusRun = $derived(status && "run" in status ? detail?.runs.find((r) => r.id === status.run) : undefined);
  const asks = $derived<Ask[]>(status?.kind === "asking" ? (app.overview?.asks ?? []).filter((a) => a.run === status.run) : []);
  // A run that finishes while you're watching gets a perked-up fox on its
  // review card; one that was already waiting when you opened the task doesn't.
  let lastKind: string | undefined;
  let justFinished = $state(false);
  $effect(() => {
    const kind = status?.kind;
    if (lastKind === "running" && kind === "review") justFinished = true;
    else if (kind !== "review") justFinished = false;
    lastKind = kind;
  });
  const history = $derived((detail?.runs ?? []).filter((r) => r.id !== statusRun?.id));
  const constraints = $derived((detail?.brief.included ?? []).filter((i) => i.kind !== "task"));
  const agents = $derived(app.overview?.agents ?? []);
  const repo = $derived(app.overview?.repo);

  export async function start() {
    if (starting || !detail) return;
    if (!repo?.trusted) {
      app.notify(t("start.untrusted"), "bad");
      return;
    }
    starting = true;
    try {
      await api.startRun(id, app.prefs.agent, app.prefs.policy, note);
      note = "";
      app.notify(t("start.onIt", { agent: app.prefs.agent }));
      app.changed();
    } catch (e) {
      app.notify(errorText(e), "bad");
    } finally {
      starting = false;
    }
  }

  export async function stop() {
    if (status?.kind === "running" || status?.kind === "asking") {
      await api.stopRun(status.run).catch((e) => app.notify(errorText(e), "bad"));
      app.notify(t("start.stopAsked"));
    }
  }

  export function accept() {
    void reviewCard?.accept(true);
  }
  export function discard() {
    void reviewCard?.discard();
  }
  export function continueWork() {
    reviewCard?.continueWork();
  }

  async function reply(ask: Ask, option: string) {
    try {
      await api.answerAsk(ask.id, option);
      app.changed();
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }

  async function answerQuestion(qid: string) {
    const a = (answer[qid] ?? "").trim();
    if (!a) return;
    try {
      await api.answerQuestion(qid, a);
      answer[qid] = "";
      app.notify(t("question.answered"), "ok");
      app.changed();
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }

  async function discardRun(r: Run) {
    await api.discard(r.id).catch((e) => app.notify(errorText(e), "bad"));
  }

  async function continueFrom(r: Run) {
    try {
      await api.startRun(id, app.prefs.agent, app.prefs.policy, note, r.id);
      app.notify(t("start.continuing", { run: r.id }));
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }

  const kindLabel = (k: string) =>
    k === "decision" ? t("kind.decision") : k === "question" ? t("kind.question") : k === "memory" ? t("kind.memory") : k === "code" ? t("kind.code") : k;
</script>

{#if error && !detail}
  <div class="page"><p class="tone-bad">{error}</p></div>
{:else if detail}
  <article class="page">
    <div class="crumb hint mono">{detail.task.path}</div>
    <h1>{detail.task.title}</h1>
    {#if view}
      <div class="status">
        <Glyph attention={view.attention} status={view.status} />
        <span>{statusText(view)}</span>
      </div>
    {/if}

    <!-- The one thing to do next. -->
    <div class="action">
      {#if detail.task.state !== "open"}
        <div class="card quiet-card">{t(detail.task.state === "done" ? "task.isDone" : "task.isDropped")}</div>
      {:else if status?.kind === "asking"}
        {#each asks as ask (ask.id)}
          <div class="card ask">
            <div class="ask-q">{t("ask.wants", { agent: statusRun?.agent ?? t("ask.theAgent") })} <strong>{@html inline(ask.request.title)}</strong></div>
            {#if ask.request.locations?.length}<div class="hint mono">{ask.request.locations.map((l) => l.path).join(", ")}</div>{/if}
            <div class="row">
              {#each ask.request.options as o, i (o.optionId)}
                <button class="btn {o.kind?.startsWith('allow') ? 'primary' : ''}" onclick={() => reply(ask, o.optionId)}>{o.name ?? o.optionId} <kbd>{i + 1}</kbd></button>
              {/each}
            </div>
            <div class="hint">{t("ask.notSandbox")}</div>
          </div>
        {/each}
        {#if statusRun}<div class="card"><RunActivity runId={statusRun.id} live compact /></div>{/if}
      {:else if status?.kind === "running"}
        <div class="card">
          <div class="card-head run-head">
            <span class="working">
              <Fox state={status.stopping ? "idle" : "working"} size={32} />
              <span class="pill work">{status.stopping ? t("run.stopping") : t("run.working")}</span>
            </span>
            <button class="btn" onclick={stop} disabled={status.stopping}>{t("run.stop")} <kbd>s</kbd></button>
          </div>
          <RunActivity runId={status.run} live />
        </div>
      {:else if status?.kind === "review" && statusRun}
        {#each detail.questions.filter((q) => q.open) as q (q.id)}
          <div class="card open-q"><span class="pill warn">{t("question.open")}</span> {q.title} <span class="hint">{t("question.openReview")}</span></div>
        {/each}
        <ReviewCard bind:this={reviewCard} run={statusRun} taskId={id} fresh={justFinished} />
      {:else if (status?.kind === "failed" || status?.kind === "interrupted") && statusRun}
        <div class="card">
          <div class="card-head">
            <span class="pill bad">{status.kind === "failed" ? t("run.failed") : t("run.interrupted")}</span>
            <span class="hint">{statusRun.agent} · {i18n.ago(statusRun.created_at)}</span>
          </div>
          <p class="detail">{status.kind === "failed" ? status.detail : t("run.interruptedBody")}</p>
          <RunActivity runId={statusRun.id} compact />
          <div class="row">
            {#if statusRun.snapshot}<button class="btn primary" onclick={() => continueFrom(statusRun)}>{t("run.continueFrom")}</button>{/if}
            <button class="btn" onclick={start}>{t("run.startOver")}</button>
            <span class="spacer"></span>
            <button class="btn quiet danger" onclick={() => discardRun(statusRun)}>{t("run.discard")}</button>
          </div>
        </div>
      {:else if status?.kind === "blocked_by_question"}
        {#each detail.questions.filter((q) => q.open) as q (q.id)}
          <div class="card">
            <div class="card-head"><span class="pill warn">{t("question.open")}</span></div>
            <div class="q-title">{q.title}</div>
            {#if q.body.trim()}<div class="prose small">{@html render(q.body)}</div>{/if}
            <textarea
              class="field"
              rows="3"
              placeholder={t("question.placeholder")}
              bind:value={answer[q.id]}
              onkeydown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) answerQuestion(q.id);
              }}
            ></textarea>
            <div class="row"><button class="btn primary" onclick={() => answerQuestion(q.id)}>{t("question.answer")} <kbd>⌘⏎</kbd></button></div>
          </div>
        {/each}
      {:else if status?.kind === "blocked_by_tasks"}
        <div class="card">
          {t("blocked.waitsFor")}
          {#each status.tasks as dep, i (dep)}
            <button class="link" onclick={() => app.openTask(dep)}>{app.task(dep)?.title ?? dep}</button>{i < status.tasks.length - 1 ? ", " : ""}
          {/each}.
        </div>
      {:else}
        <div class="card start">
          <div class="agents" role="radiogroup" aria-label="Agent">
            {#each agents as a (a.name)}
              <button
                class="agent press"
                class:on={app.prefs.agent === a.name}
                role="radio"
                aria-checked={app.prefs.agent === a.name}
                title={a.command}
                onclick={() => {
                  app.prefs.agent = a.name;
                  app.savePrefs();
                }}>{a.name}</button
              >
            {/each}
          </div>
          <textarea class="field" rows="2" placeholder={t("start.note")} bind:value={note}></textarea>
          <div class="row">
            <button class="btn primary" disabled={starting} onclick={start}>{t("start.start", { agent: app.prefs.agent })} <kbd>r</kbd></button>
            <label class="policy">
              <input
                type="checkbox"
                checked={app.prefs.policy === "auto"}
                onchange={(e) => {
                  app.prefs.policy = (e.currentTarget as HTMLInputElement).checked ? "auto" : "ask";
                  app.savePrefs();
                }}
              />
              {t("start.auto")}
            </label>
          </div>
          {#if detail.dirty_checkout}
            <div class="hint">{t("start.dirty")}</div>
          {/if}
        </div>
      {/if}
    </div>

    {#if detail.task.body.trim()}
      <div class="section-title">{t("section.about")}</div>
      <div class="prose">{@html render(detail.task.body)}</div>
    {/if}

    <div class="section-title">{t("section.told")} <span class="size">{t("told.size", tokenParams(estimateTokens(detail.brief.markdown)))}</span></div>
    <div class="constraints">
      {#if detail.required.length}
        <div class="c-row"><span class="c-kind">{t("told.doneWhen")}</span><span>{@html t("told.doneChecks", { checks: detail.required.map((c) => `<code>${c.replace(/[<>&]/g, "")}</code>`).join(", ") })}</span></div>
      {:else}
        <div class="c-row"><span class="c-kind">{t("told.doneWhen")}</span><span>{t("told.doneNoChecks")}</span></div>
      {/if}
      {#each constraints as c (c.kind + c.id)}
        <button class="c-row link-row" onclick={() => app.go({ kind: "rules" })}>
          <span class="c-kind">{kindLabel(c.kind)}</span>
          <span class="c-text">
            <span class="c-title">{c.title}</span><span class="hint mono"> {c.id}</span><span class="why hint">{c.why}</span>
            <!-- A rule a check enforces, or prose nobody is held to. -->
            {#if c.enforced_by?.length}
              <span class="why enforced">{t("told.enforcedBy", { checks: i18n.list(c.enforced_by) })}</span>
            {:else if c.enforced_by}
              <span class="why noted">{t("told.note")}</span>
            {/if}
          </span>
        </button>
      {/each}
      {#each detail.brief.problems as p (p)}
        <div class="c-row"><span class="c-kind tone-bad">{t("told.unreadable")}</span><span class="tone-bad">{p}</span></div>
      {/each}
      {#if detail.brief.omitted.length}
        <div class="c-row"><span class="c-kind">{t("told.leftOut")}</span><span class="hint">{t("told.leftOutBody", { ids: detail.brief.omitted.map((o) => o.id).join(", ") })}</span></div>
      {/if}
      <button class="btn quiet show-brief" onclick={() => (showBrief = !showBrief)}>{showBrief ? t("told.hideBrief") : t("told.showBrief")}</button>
      {#if showBrief}<pre class="brief scroll">{detail.brief.markdown}</pre>{/if}
    </div>

    {#if history.length}
      <div class="section-title">{t("section.attempts")}</div>
      <div class="history">
        {#each history as r (r.id)}
          <div class="attempt">
            <button class="attempt-head press" onclick={() => (openRun = openRun === r.id ? null : r.id)}>
              <span class="mono">{r.id}</span>
              <span>{r.agent}</span>
              <span class="pill {r.resolution === 'accepted' ? 'ok' : r.state === 'failed' ? 'bad' : 'dim'}">{runWord(r.state, r.stop_reason, r.resolution)}</span>
              <span class="hint">{i18n.ago(r.created_at)}</span>
            </button>
            {#if openRun === r.id}
              <div class="attempt-body"><RunActivity runId={r.id} /></div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </article>
{/if}

<style>
  .size {
    margin-left: 6px;
    font-weight: 450;
    text-transform: none;
    letter-spacing: 0;
  }
  .page {
    max-width: 840px;
    margin: 0 auto;
    padding: var(--s7) var(--s7) var(--s8);
  }
  .crumb {
    margin-bottom: var(--s2);
  }
  h1 {
    margin: 0 0 var(--s2);
    font-size: 26px;
    line-height: 1.25;
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .status {
    display: flex;
    align-items: center;
    gap: var(--s2);
    color: var(--muted);
  }
  .action {
    display: flex;
    flex-direction: column;
    gap: var(--s4);
    margin-top: var(--s5);
  }
  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s2);
    margin-bottom: var(--s3);
  }
  .working {
    display: flex;
    align-items: center;
    gap: var(--s3);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s3);
    margin-top: var(--s3);
  }
  .spacer {
    flex: 1;
  }
  .ask {
    border-color: var(--warn);
    box-shadow: 0 0 0 3px var(--warn-soft);
    display: flex;
    flex-direction: column;
    gap: var(--s2);
  }
  .ask-q {
    font-size: 15px;
  }
  .start {
    display: flex;
    flex-direction: column;
    gap: var(--s4);
  }
  .agents {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s2);
  }
  .agent {
    height: 28px;
    padding: 0 var(--s3);
    border-radius: 14px;
    border: 1px solid var(--line);
    font-size: 13px;
  }
  .agent:hover {
    background: var(--hover);
  }
  .agent.on {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }
  .policy {
    display: flex;
    align-items: center;
    gap: var(--s2);
    color: var(--muted);
    font-size: 13px;
  }
  .open-q {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: baseline;
    border-color: var(--warn);
  }
  .quiet-card {
    color: var(--muted);
  }
  .detail {
    margin: 0 0 10px;
  }
  .q-title {
    font-weight: 600;
    margin-bottom: 6px;
  }
  .small {
    font-size: 13px;
    color: var(--muted);
    margin-bottom: 8px;
  }
  .link {
    color: var(--accent);
    font-weight: 500;
  }
  .constraints {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .c-row {
    display: flex;
    gap: var(--s4);
    padding: var(--s3) 0;
    border-bottom: 1px solid var(--line);
    text-align: left;
    font-size: 13.5px;
  }
  .c-title {
    transition: color var(--fast) var(--ease);
  }
  .link-row:hover .c-title {
    color: var(--accent);
  }
  .c-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-width: var(--measure);
  }
  .c-title {
    font-weight: 500;
  }
  .why {
    font-size: 12.5px;
  }
  /* Enforced says a check must pass, not that it did: no green here. */
  .enforced {
    color: var(--muted);
  }
  .noted {
    color: var(--warn);
    font-style: italic;
  }
  .c-kind {
    flex: none;
    width: 104px;
    color: var(--faint);
    font-size: 12.5px;
  }
  .show-brief {
    align-self: flex-start;
    margin-top: var(--s3);
    margin-left: calc(-1 * var(--s2));
    height: 28px;
    padding: 0 var(--s2);
    font-size: 12.5px;
  }
  .brief {
    max-height: 420px;
    margin: var(--s2) 0 0;
    padding: var(--s4);
    border-radius: 10px;
    background: var(--rail);
    white-space: pre-wrap;
    animation: enter var(--quick) var(--ease);
  }
  .history {
    display: flex;
    flex-direction: column;
    gap: var(--s2);
  }
  .attempt {
    border: 1px solid var(--line);
    border-radius: 10px;
    overflow: hidden;
  }
  .attempt-head {
    --press: 0.99;
    display: flex;
    align-items: center;
    gap: var(--s3);
    width: 100%;
    padding: var(--s3) var(--s4);
    text-align: left;
    font-size: 13px;
  }
  .attempt-head:focus-visible {
    outline-offset: -2px;
  }
  .attempt-head:hover {
    background: var(--hover);
  }
  .attempt-body {
    padding: var(--s1) var(--s4) var(--s4);
    animation: enter var(--quick) var(--ease);
  }
</style>
