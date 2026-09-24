<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { api, errorKind, errorText } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { checkWord, spent, tokenParams } from "../lib/status";
  import type { Accepted, Review, Run } from "../lib/types";
  import RunActivity from "./RunActivity.svelte";

  let { run, taskId }: { run: Run; taskId: string } = $props();

  let review = $state<Review | null>(null);
  let error = $state<string | null>(null);
  let busy = $state<"" | "accept" | "discard" | "continue">("");
  let result = $state<Accepted | null>(null);
  let noting = $state(false);
  let note = $state("");
  let approved = $state(false);
  let showActivity = $state(false);

  $effect(() => {
    void app.tick;
    const id = run.id;
    api
      .review(id)
      .then((r) => {
        review = r;
        error = null;
      })
      .catch((e) => (error = errorText(e)));
  });

  const total = $derived(review ? review.files.reduce((a, f) => [a[0]! + (f.added ?? 0), a[1]! + (f.removed ?? 0)], [0, 0]) : [0, 0]);
  const allPass = $derived(!!review && review.checks.every(([, s]) => checkWord(s).tone === "ok"));
  const anyFail = $derived(!!review && review.checks.some(([, s]) => checkWord(s).tone === "bad"));
  const needsApproval = $derived(!!review?.approval_token && !approved);
  const empty = $derived(!!review && review.files.length === 0);

  export async function accept(closeTask = true) {
    if (!review || busy || empty) return;
    if (needsApproval) {
      app.notify(t("review.approveFirst"), "info");
      return;
    }
    busy = "accept";
    try {
      result = await api.accept(run.id, closeTask, approved ? (review.approval_token ?? undefined) : undefined);
      if (result.result === "applied") app.notify(closeTask ? t("review.acceptedClosed") : t("review.accepted"), "ok");
    } catch (e) {
      app.notify(errorText(e), "bad");
    } finally {
      busy = "";
    }
  }

  export async function discard() {
    if (busy) return;
    busy = "discard";
    try {
      await api.discard(run.id);
      app.notify(t("review.discarded"));
    } catch (e) {
      app.notify(errorText(e), "bad");
    } finally {
      busy = "";
    }
  }

  export function continueWork() {
    noting = true;
  }

  async function startContinue() {
    busy = "continue";
    try {
      await api.startRun(taskId, app.prefs.agent, app.prefs.policy, note, run.id);
      noting = false;
      note = "";
      app.notify(t("review.continuing", { agent: app.prefs.agent }));
    } catch (e) {
      app.notify(errorKind(e) === "denied" ? errorText(e) : t("review.couldNotStart", { error: errorText(e) }), "bad");
    } finally {
      busy = "";
    }
  }
</script>

<div class="card review">
  <div class="head">
    <div>
      <div class="title">
        {t(run.stop_reason === "cancelled" ? "review.stopped" : "review.finished", { agent: run.agent })}
        {#if review}<span class="hint">· {t("review.files", { n: review.files.length })} · <span class="tone-ok">+{total[0]}</span> <span class="tone-bad">−{total[1]}</span> · <span class="mono">{t("review.onto", { branch: review.target })}</span>{#if spent(run.usage) != null}{" · "}{t("review.tokens", tokenParams(spent(run.usage)!))}{/if}</span>{/if}
      </div>
      {#if empty}
        <div class="verdict hint">{t("review.empty")}</div>
      {:else if review && review.checks.length}
        <div class="verdict {allPass ? 'tone-ok' : anyFail ? 'tone-bad' : 'tone-warn'}">
          {allPass ? t("review.allPass") : anyFail ? t("review.fails") : t("review.pending")}
        </div>
      {:else if review}
        <div class="verdict hint">{t("review.noChecks")}</div>
      {/if}
    </div>
    <button class="btn quiet" onclick={() => (showActivity = !showActivity)}>{showActivity ? t("review.hide") : t("review.whatItDid")}</button>
  </div>

  {#if error}<div class="tone-bad">{error}</div>{/if}

  {#if showActivity}
    <div class="activity"><RunActivity runId={run.id} /></div>
  {/if}

  {#if review && !empty}
    <div class="files">
      {#each review.files as f (f.path)}
        <button class="file" onclick={() => app.go({ kind: "diff", run: run.id, path: f.path })}>
          <span class="mono path">{f.path}</span>
          {#if review.protected.includes(f.path)}<span class="pill warn">{t("review.protected")}</span>{/if}
          <span class="n mono">{#if f.added === null}{t("review.binary")}{:else}<span class="tone-ok">+{f.added}</span> <span class="tone-bad">−{f.removed}</span>{/if}</span>
        </button>
      {/each}
    </div>

    {#if review.checks.length}
      <div class="checks">
        {#each review.checks as [name, s] (name)}
          {@const w = checkWord(s)}
          <div class="check"><span class="mono">{name}</span><span class="pill {w.tone}">{w.word}</span>{#if w.hint}<span class="hint">{w.hint}</span>{/if}</div>
        {/each}
      </div>
    {/if}

    {#if review.approval_token}
      <label class="approve">
        <input type="checkbox" bind:checked={approved} />
        <span>{t("review.approve", { paths: review.protected.join(", ") })} <strong>{t("review.approveStrong")}</strong></span>
      </label>
    {/if}
  {/if}

  {#if result && result.result !== "applied"}
    <div class="outcome">
      {#if result.result === "conflict"}
        <span class="tone-bad">{t("review.conflict", { branch: review?.target ?? "", paths: result.paths.join(", ") })}</span>
      {:else if result.result === "checks_failed"}
        <span class="tone-bad">{t("review.checksFailed", { checks: result.failing.join(", "), branch: review?.target ?? "" })}</span>
      {:else if result.result === "needs_approval"}
        <span class="tone-warn">{t("review.needsApproval", { paths: result.paths.join(", ") })}</span>
      {/if}
    </div>
  {/if}

  {#if noting}
    <div class="continue">
      <textarea class="field" rows="2" placeholder={t("review.notePlaceholder")} bind:value={note}></textarea>
      <div class="row">
        <button class="btn primary" disabled={busy !== ""} onclick={startContinue}>{t("review.continueWith", { agent: app.prefs.agent })}</button>
        <button class="btn quiet" onclick={() => (noting = false)}>{t("review.cancel")}</button>
      </div>
    </div>
  {:else}
    <div class="actions">
      {#if !empty}
        <button class="btn primary" disabled={!review || busy !== "" || needsApproval} onclick={() => accept(true)}>{t("review.acceptClose")} <kbd>a</kbd></button>
        <button class="btn" disabled={!review || busy !== "" || needsApproval} onclick={() => accept(false)}>{t("review.acceptKeep")}</button>
      {/if}
      <button class="btn" disabled={busy !== ""} onclick={continueWork}>{t("review.continue")} <kbd>c</kbd></button>
      <span class="spacer"></span>
      <button class="btn quiet danger" disabled={busy !== ""} onclick={discard}>{t("review.discard")} <kbd>x</kbd></button>
    </div>
  {/if}
</div>

<style>
  .review {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 12px;
  }
  .title {
    font-weight: 600;
    font-size: 15px;
  }
  .verdict {
    margin-top: 2px;
    font-size: 13px;
  }
  .files {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--line);
    border-radius: 10px;
    overflow: hidden;
  }
  .file {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    text-align: left;
    border-bottom: 1px solid var(--line);
  }
  .file:last-child {
    border-bottom: none;
  }
  .file:hover {
    background: var(--hover);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .n {
    font-size: 12px;
  }
  .checks {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
  }
  .check .mono {
    min-width: 110px;
  }
  .approve {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--warn-soft);
    font-size: 13px;
  }
  .approve input {
    margin-top: 3px;
  }
  .outcome {
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--rail);
    font-size: 13px;
  }
  .actions,
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
  .continue {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .activity {
    padding: 12px;
    border-radius: 10px;
    background: var(--bg);
    border: 1px solid var(--line);
  }
</style>
