<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { api, errorText } from "../lib/api";
  import { ago, runWord } from "../lib/format";
  import { inline, render } from "../lib/md";
  import type { Ask, Run, TaskDetail } from "../lib/types";
  import Glyph from "./Glyph.svelte";
  import ReviewCard from "./ReviewCard.svelte";
  import RunActivity from "./RunActivity.svelte";

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
  const history = $derived((detail?.runs ?? []).filter((r) => r.id !== statusRun?.id));
  const constraints = $derived((detail?.brief.included ?? []).filter((i) => i.kind !== "task"));
  const agents = $derived(app.overview?.agents ?? []);
  const repo = $derived(app.overview?.repo);

  export async function start() {
    if (starting || !detail) return;
    if (!repo?.trusted) {
      app.notify("Trust this repository first: agents and checks run its code.", "bad");
      return;
    }
    starting = true;
    try {
      await api.startRun(id, app.prefs.agent, app.prefs.policy, note);
      note = "";
      app.notify(`${app.prefs.agent} is on it.`);
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
      app.notify("Asked the agent to stop.");
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
      app.notify("Answered. It's in the question file, so the next agent sees it.", "ok");
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
      app.notify(`Continuing from ${r.id}.`);
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }

  const kindLabel: Record<string, string> = { invariant: "Must hold", decision: "Decision", question: "Question" };
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
        <span>{view.reason}</span>
      </div>
    {/if}

    <!-- The one thing to do next. -->
    <div class="action">
      {#if detail.task.state !== "open"}
        <div class="card quiet-card">This task is {detail.task.state}. Its history is below.</div>
      {:else if status?.kind === "asking"}
        {#each asks as ask (ask.id)}
          <div class="card ask">
            <div class="ask-q"><strong>{statusRun?.agent ?? "The agent"}</strong> wants to: <strong>{@html inline(ask.request.title)}</strong></div>
            {#if ask.request.locations?.length}<div class="hint mono">{ask.request.locations.map((l) => l.path).join(", ")}</div>{/if}
            <div class="row">
              {#each ask.request.options as o, i (o.optionId)}
                <button class="btn {o.kind?.startsWith('allow') ? 'primary' : ''}" onclick={() => reply(ask, o.optionId)}>{o.name ?? o.optionId} <kbd>{i + 1}</kbd></button>
              {/each}
            </div>
            <div class="hint">This is an approval, not a sandbox: the agent runs with your user's permissions.</div>
          </div>
        {/each}
        {#if statusRun}<div class="card"><RunActivity runId={statusRun.id} live compact /></div>{/if}
      {:else if status?.kind === "running"}
        <div class="card">
          <div class="card-head">
            <span class="pill work">{status.stopping ? "stopping" : "working"}</span>
            <button class="btn" onclick={stop} disabled={status.stopping}>Stop <kbd>s</kbd></button>
          </div>
          <RunActivity runId={status.run} live />
        </div>
      {:else if status?.kind === "review" && statusRun}
        {#each detail.questions.filter((q) => q.open) as q (q.id)}
          <div class="card open-q"><span class="pill warn">open question</span> {q.title} <span class="hint">It was asked about this task and isn't answered yet. Accepting now decides it by default.</span></div>
        {/each}
        <ReviewCard bind:this={reviewCard} run={statusRun} taskId={id} />
      {:else if (status?.kind === "failed" || status?.kind === "interrupted") && statusRun}
        <div class="card">
          <div class="card-head">
            <span class="pill bad">{status.kind === "failed" ? "failed" : "interrupted"}</span>
            <span class="hint">{statusRun.agent} · {ago(statusRun.created_at)}</span>
          </div>
          <p class="detail">{status.kind === "failed" ? status.detail : "Kitsu's worker for this run disappeared (crash or kill). What the agent did up to then is kept."}</p>
          <RunActivity runId={statusRun.id} compact />
          <div class="row">
            {#if statusRun.snapshot}<button class="btn primary" onclick={() => continueFrom(statusRun)}>Continue from its work</button>{/if}
            <button class="btn" onclick={start}>Start over</button>
            <span class="spacer"></span>
            <button class="btn quiet danger" onclick={() => discardRun(statusRun)}>Discard</button>
          </div>
        </div>
      {:else if status?.kind === "blocked_by_question"}
        {#each detail.questions.filter((q) => q.open) as q (q.id)}
          <div class="card">
            <div class="card-head"><span class="pill warn">open question</span></div>
            <div class="q-title">{q.title}</div>
            {#if q.body.trim()}<div class="prose small">{@html render(q.body)}</div>{/if}
            <textarea
              class="field"
              rows="3"
              placeholder="Your answer. It's written into the question file, so every future agent sees it."
              bind:value={answer[q.id]}
              onkeydown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) answerQuestion(q.id);
              }}
            ></textarea>
            <div class="row"><button class="btn primary" onclick={() => answerQuestion(q.id)}>Answer <kbd>⌘⏎</kbd></button></div>
          </div>
        {/each}
      {:else if status?.kind === "blocked_by_tasks"}
        <div class="card">
          Waits for
          {#each status.tasks as t, i (t)}
            <button class="link" onclick={() => app.openTask(t)}>{app.task(t)?.title ?? t}</button>{i < status.tasks.length - 1 ? ", " : ""}
          {/each}.
        </div>
      {:else}
        <div class="card start">
          <div class="agents" role="radiogroup" aria-label="Agent">
            {#each agents as a (a.name)}
              <button
                class="agent"
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
          <textarea class="field" rows="2" placeholder="Anything to add for the agent? (optional — the task, rules and history are already in its brief)" bind:value={note}></textarea>
          <div class="row">
            <button class="btn primary" disabled={starting} onclick={start}>Start {app.prefs.agent} <kbd>r</kbd></button>
            <label class="policy">
              <input
                type="checkbox"
                checked={app.prefs.policy === "auto"}
                onchange={(e) => {
                  app.prefs.policy = (e.currentTarget as HTMLInputElement).checked ? "auto" : "ask";
                  app.savePrefs();
                }}
              />
              Let it run commands in its worktree without asking
            </label>
          </div>
          {#if detail.dirty_checkout}
            <div class="hint">You have uncommitted changes. The agent starts from your last commit and won't see them.</div>
          {/if}
        </div>
      {/if}
    </div>

    {#if detail.task.body.trim()}
      <div class="section-title">About</div>
      <div class="prose">{@html render(detail.task.body)}</div>
    {/if}

    <div class="section-title">What the agent is told</div>
    <div class="constraints">
      {#if detail.task.checks.length}
        <div class="c-row"><span class="c-kind">Done when</span><span>{#each detail.task.checks as c, i (c)}<code>{c}</code>{i < detail.task.checks.length - 1 ? ", " : ""}{/each} pass{detail.task.checks.length === 1 ? "es" : ""}, and you accept it</span></div>
      {:else}
        <div class="c-row"><span class="c-kind">Done when</span><span>you accept it (no checks defined)</span></div>
      {/if}
      {#each constraints as c (c.kind + c.id)}
        <button class="c-row link-row" onclick={() => app.go({ kind: "rules" })}>
          <span class="c-kind">{kindLabel[c.kind] ?? c.kind}</span>
          <span class="c-text"><span class="c-title">{c.title}</span><span class="hint mono"> {c.id}</span><span class="why hint">{c.why}</span></span>
        </button>
      {/each}
      {#each detail.brief.problems as p (p)}
        <div class="c-row"><span class="c-kind tone-bad">Unreadable</span><span class="tone-bad">{p}</span></div>
      {/each}
      {#if detail.brief.omitted.length}
        <div class="c-row"><span class="c-kind">Left out</span><span class="hint">{detail.brief.omitted.map((o) => o.id).join(", ")} (over budget; the agent can read them)</span></div>
      {/if}
      <button class="btn quiet show-brief" onclick={() => (showBrief = !showBrief)}>{showBrief ? "Hide" : "Show"} the full brief</button>
      {#if showBrief}<pre class="brief scroll">{detail.brief.markdown}</pre>{/if}
    </div>

    {#if history.length}
      <div class="section-title">Earlier attempts</div>
      <div class="history">
        {#each history as r (r.id)}
          <div class="attempt">
            <button class="attempt-head" onclick={() => (openRun = openRun === r.id ? null : r.id)}>
              <span class="mono">{r.id}</span>
              <span>{r.agent}</span>
              <span class="pill {r.resolution === 'accepted' ? 'ok' : r.state === 'failed' ? 'bad' : 'dim'}">{r.resolution ?? runWord(r.state, r.stop_reason)}</span>
              <span class="hint">{ago(r.created_at)}</span>
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
  .page {
    max-width: 820px;
    margin: 0 auto;
    padding: 34px 40px 80px;
  }
  .crumb {
    margin-bottom: 6px;
  }
  h1 {
    margin: 0 0 8px;
    font-size: 26px;
    line-height: 1.25;
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
  }
  .action {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: 22px;
  }
  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 10px;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    margin-top: 10px;
  }
  .spacer {
    flex: 1;
  }
  .ask {
    border-color: var(--warn);
    box-shadow: 0 0 0 3px var(--warn-soft);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .ask-q {
    font-size: 15px;
  }
  .start {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .agents {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .agent {
    height: 28px;
    padding: 0 12px;
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
    gap: 6px;
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
    gap: 14px;
    padding: 7px 0;
    border-bottom: 1px solid var(--line);
    text-align: left;
    font-size: 13.5px;
  }
  .link-row:hover .c-title {
    color: var(--accent);
  }
  .c-text {
    display: flex;
    flex-direction: column;
  }
  .c-title {
    font-weight: 500;
  }
  .why {
    font-size: 12.5px;
  }
  .c-kind {
    flex: none;
    width: 92px;
    color: var(--faint);
    font-size: 12.5px;
  }
  .show-brief {
    align-self: flex-start;
    margin-top: 8px;
    height: 26px;
    padding: 0 8px;
    font-size: 12.5px;
  }
  .brief {
    max-height: 420px;
    margin: 8px 0 0;
    padding: 14px 16px;
    border-radius: 10px;
    background: var(--rail);
    white-space: pre-wrap;
  }
  .history {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .attempt {
    border: 1px solid var(--line);
    border-radius: 10px;
    overflow: hidden;
  }
  .attempt-head {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 8px 12px;
    text-align: left;
    font-size: 13px;
  }
  .attempt-head:hover {
    background: var(--hover);
  }
  .attempt-body {
    padding: 4px 12px 12px;
  }
</style>
