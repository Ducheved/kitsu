<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { api, errorText } from "../lib/api";
  import { checkWord } from "../lib/format";
  import { render } from "../lib/md";
  import type { Rules } from "../lib/types";

  let rules = $state<Rules | null>(null);
  let error = $state<string | null>(null);
  let running = $state(false);
  let answers = $state<Record<string, string>>({});

  $effect(() => {
    void app.tick;
    api
      .rules()
      .then((r) => {
        rules = r;
        error = null;
      })
      .catch((e) => (error = errorText(e)));
  });

  export async function runChecks() {
    if (running) return;
    running = true;
    try {
      const out = await api.runChecks();
      const failed = out.filter((e) => e.outcome !== "pass");
      app.notify(failed.length ? `${failed.length} of ${out.length} checks fail: ${failed.map((f) => f.check_name).join(", ")}` : `All ${out.length} checks pass.`, failed.length ? "bad" : "ok");
      app.changed();
    } catch (e) {
      app.notify(errorText(e), "bad");
    } finally {
      running = false;
    }
  }

  async function answer(id: string) {
    const a = (answers[id] ?? "").trim();
    if (!a) return;
    try {
      await api.answerQuestion(id, a);
      answers[id] = "";
      app.changed();
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }
</script>

<article class="page">
  <h1>Rules</h1>
  <p class="lede">What has to stay true, what was decided, and what's still unknown. Agents get the ones that touch their task, with the reason. They live in <span class="mono">.kitsu/</span> and change through review like code.</p>
  {#if error}<p class="tone-bad">{error}</p>{/if}

  {#if rules}
    <div class="section-title">Must hold</div>
    {#each rules.invariants.filter((i) => i.active) as inv (inv.id)}
      <div class="item">
        <div class="item-head">
          <span class="item-title">{inv.title}</span>
          {#each inv.checks as c (c.name)}
            {@const w = checkWord(c.status)}
            <span class="pill {w.tone}" title={w.hint ?? ""}>{c.name} {w.word}</span>
          {:else}
            <span class="pill dim" title="No check enforces this. Reviewers and agents see it; nothing verifies it.">review only</span>
          {/each}
        </div>
        {#if inv.body.trim()}<div class="prose small">{@html render(inv.body)}</div>{/if}
        <div class="hint mono">{inv.scope.length ? inv.scope.join(", ") : "whole repository"}{inv.decision ? ` · from ${inv.decision}` : ""}</div>
      </div>
    {:else}
      <p class="hint">No invariants yet. They're for the things an agent will get wrong if nobody tells it: ownership, ordering, retry rules.</p>
    {/each}

    <div class="section-title">Decisions</div>
    {#each rules.decisions as d (d.id)}
      <div class="item">
        <div class="item-head"><span class="item-title">{d.title}</span>{#if d.state !== "accepted"}<span class="pill {d.state === 'proposed' ? 'warn' : 'dim'}">{d.state}</span>{/if}</div>
        {#if d.body.trim()}<div class="prose small">{@html render(d.body)}</div>{/if}
        {#if d.rejected.length}
          <ul class="rejected">
            {#each d.rejected as r (r)}<li><span class="no">not</span> {r}</li>{/each}
          </ul>
        {/if}
      </div>
    {:else}
      <p class="hint">No decisions recorded.</p>
    {/each}

    {#if rules.questions.length}
      <div class="section-title">Questions</div>
      {#each rules.questions as q (q.id)}
        <div class="item">
          <div class="item-head"><span class="item-title">{q.title}</span><span class="pill {q.open ? 'warn' : 'ok'}">{q.open ? "open" : "answered"}</span></div>
          {#if q.answer}<div class="prose small"><strong>Answer:</strong> {q.answer}</div>{/if}
          {#if q.open}
            <div class="answer-row">
              <input class="field" placeholder="Answer" bind:value={answers[q.id]} onkeydown={(e) => e.key === "Enter" && answer(q.id)} />
              <button class="btn" onclick={() => answer(q.id)}>Answer</button>
            </div>
          {/if}
          {#if q.blocks.length}<div class="hint">blocks {q.blocks.join(", ")}</div>{/if}
        </div>
      {/each}
    {/if}

    <div class="section-title checks-title">
      <span>Checks</span>
      <button class="btn" disabled={running} onclick={runChecks}>{running ? "Running…" : "Run all on your checkout"}</button>
    </div>
    {#each rules.checks as c (c.name)}
      {@const w = checkWord(c.status)}
      <div class="check">
        <span class="mono name">{c.name}</span>
        <span class="mono cmd">{c.run}</span>
        <span class="pill {w.tone}" title={w.hint ?? ""}>{w.word}</span>
      </div>
    {:else}
      <p class="hint">No checks in <span class="mono">.kitsu/kitsu.toml</span>. Without checks, "done" means only that a human said so.</p>
    {/each}
  {/if}
</article>

<style>
  .page {
    max-width: 820px;
    margin: 0 auto;
    padding: 34px 40px 80px;
  }
  h1 {
    margin: 0 0 6px;
    font-size: 26px;
    font-weight: 650;
  }
  .lede {
    margin: 0;
    color: var(--muted);
    max-width: 64ch;
  }
  .item {
    padding: 14px 0;
    border-bottom: 1px solid var(--line);
  }
  .item-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }
  .item-title {
    font-weight: 600;
    margin-right: auto;
  }
  .small {
    font-size: 13.5px;
    color: var(--muted);
  }
  .rejected {
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
    font-size: 13px;
  }
  .rejected li {
    padding: 2px 0;
    color: var(--muted);
  }
  .no {
    display: inline-block;
    width: 30px;
    color: var(--bad);
    font-weight: 600;
    font-size: 11.5px;
    text-transform: uppercase;
  }
  .answer-row {
    display: flex;
    gap: 8px;
    margin: 6px 0;
  }
  .checks-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 8px 0;
    border-bottom: 1px solid var(--line);
  }
  .name {
    width: 120px;
  }
  .cmd {
    flex: 1;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
