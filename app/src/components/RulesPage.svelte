<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { api, errorText } from "../lib/api";
  import { i18n, t } from "../lib/i18n/index.svelte";
  import { checkWord } from "../lib/status";
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
      app.notify(
        failed.length ? t("rules.someFail", { n: failed.length, total: out.length, names: i18n.list(failed.map((f) => f.check_name)) }) : t("rules.allPass", { n: out.length }),
        failed.length ? "bad" : "ok",
      );
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
  <header data-tour="rules">
    <h1>{t("rules.title")}</h1>
    <p class="lede">{t("rules.lede")}</p>
  </header>
  {#if error}<p class="tone-bad">{error}</p>{/if}

  {#if rules}
    <div class="section-title">{t("rules.mustHold")}</div>
    {#each rules.checks.filter((c) => c.guards.length) as c (c.name)}
      {@const w = checkWord(c.status)}
      <div class="item">
        <div class="item-head">
          <span class="item-title">{c.why ?? c.name}</span>
          <span class="pill {w.tone}" title={w.hint ?? ""}>{c.name} {w.word}</span>
        </div>
        <div class="hint mono">{t("rules.guards", { paths: c.guards.join(", ") })}</div>
      </div>
    {:else}
      <p class="hint">{t("rules.noGuards")}</p>
    {/each}

    <div class="section-title">{t("rules.decisions")}</div>
    {#each rules.decisions as d (d.id)}
      <div class="item">
        <div class="item-head"><span class="item-title">{d.title}</span>{#if d.state !== "accepted"}<span class="pill {d.state === 'proposed' ? 'warn' : 'dim'}">{d.state === "proposed" ? t("rules.proposed") : d.state === "superseded" ? t("rules.superseded") : d.state}</span>{/if}</div>
        {#if d.body.trim()}<div class="prose small">{@html render(d.body)}</div>{/if}
        {#if d.rejected.length}
          <ul class="rejected">
            {#each d.rejected as r (r)}<li><span class="no">{t("rules.not")}</span> {r}</li>{/each}
          </ul>
        {/if}
      </div>
    {:else}
      <p class="hint">{t("rules.noDecisions")}</p>
    {/each}

    <div class="section-title">{t("rules.memory")}</div>
    {#each rules.memory as m (m.id)}
      <div class="item">
        <div class="item-head">
          <span class="item-title">{m.title}</span>
          <span class="pill dim">{t(`memkind.${m.kind}`)}</span>
          {#if m.state === "retired"}
            <span class="pill dim" title={m.reason ?? ""}>{t("rules.memoryRetired")}</span>
          {:else if m.superseded_by}
            <span class="pill dim">{t("rules.memorySuperseded", { by: m.superseded_by })}</span>
          {:else if m.freshness?.status === "stale"}
            <span class="pill warn" title={t("rules.memoryStaleHint", { files: m.freshness.changed.join(", ") })}>{t("rules.memoryStale")}</span>
          {:else if m.freshness?.status === "uncommitted"}
            <span class="pill dim">{t("rules.memoryUncommitted")}</span>
          {/if}
        </div>
        {#if m.freshness?.status === "stale"}<div class="hint tone-warn">{t("rules.memoryStaleHint", { files: m.freshness.changed.join(", ") })}</div>{/if}
        {#if m.body.trim()}<div class="prose small">{@html render(m.body)}</div>{/if}
        <div class="hint mono">
          {#if m.personal}{t("rules.memoryPersonal")}{:else}{m.scope.length ? m.scope.join(", ") : t("rules.wholeRepo")}{#if m.anchors.length}{" · "}{t("rules.memoryAnchors", { files: m.anchors.join(", ") })}{/if}{/if}{m.by ? ` · ${m.by}` : ""}{m.run ? ` · ${m.run}` : ""}
        </div>
      </div>
    {:else}
      <p class="hint">{t("rules.memoryEmpty")}</p>
    {/each}

    {#if rules.questions.length}
      <div class="section-title">{t("rules.questions")}</div>
      {#each rules.questions as q (q.id)}
        <div class="item">
          <div class="item-head"><span class="item-title">{q.title}</span><span class="pill {q.open ? 'warn' : 'ok'}">{q.open ? t("rules.open") : t("rules.answered")}</span></div>
          {#if q.answer}<div class="prose small"><strong>{t("rules.answerLabel")}</strong> {q.answer}</div>{/if}
          {#if q.open}
            <div class="answer-row">
              <input class="field" placeholder={t("question.answer")} bind:value={answers[q.id]} onkeydown={(e) => e.key === "Enter" && answer(q.id)} />
              <button class="btn" onclick={() => answer(q.id)}>{t("question.answer")}</button>
            </div>
          {/if}
          {#if q.blocks.length}<div class="hint">{t("rules.blocks", { tasks: i18n.list(q.blocks) })}</div>{/if}
        </div>
      {/each}
    {/if}

    <div class="section-title checks-title">
      <span>{t("rules.checks")}</span>
      <button class="btn" disabled={running} onclick={runChecks}>{running ? t("rules.running") : t("rules.runAll")}</button>
    </div>
    {#each rules.checks as c (c.name)}
      {@const w = checkWord(c.status)}
      <div class="check">
        <span class="mono name">{c.name}</span>
        <span class="mono cmd">{c.run}</span>
        <span class="pill {w.tone}" title={w.hint ?? ""}>{w.word}</span>
      </div>
    {:else}
      <p class="hint">{t("rules.noChecks")}</p>
    {/each}
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
    margin: 0;
    color: var(--muted);
    max-width: 64ch;
    line-height: 1.65;
  }
  .item {
    padding: var(--s4) 0 var(--s4);
    border-bottom: 1px solid var(--line);
  }
  .item-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s2);
    margin-bottom: var(--s2);
  }
  .item-title {
    font-weight: 600;
    margin-right: auto;
  }
  .small {
    font-size: 13.5px;
    color: var(--muted);
  }
  .item > .hint {
    margin-top: var(--s1);
  }
  .rejected {
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
    font-size: 13px;
  }
  .rejected li {
    padding: 3px 0;
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
    gap: var(--s2);
    margin: var(--s2) 0;
  }
  .checks-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--s4);
    padding: var(--s3) 0;
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
