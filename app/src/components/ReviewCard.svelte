<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { errorKind, errorText } from "../lib/api";
  import { i18n, t } from "../lib/i18n/index.svelte";
  import { render } from "../lib/md";
  import { checkWord, receiptCommand, receiptLine, spent, tokenParams } from "../lib/status";
  import type { Accepted, Judgment, Review, Run } from "../lib/types";
  import Fox from "./Fox.svelte";
  import RunActivity from "./RunActivity.svelte";
  // This project's commands, fixed for as long as the component lives.
  const api = app.api;

  let { run, taskId, fresh = false }: { run: Run; taskId: string; fresh?: boolean } = $props();

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
  // A check is green only next to its receipt (see checkWord).
  const receiptOf = (name: string) => review?.receipts.find((r) => r.check === name) ?? null;
  const allPass = $derived(!!review && review.checks.every(([n, s]) => checkWord(s, receiptOf(n)).tone === "ok"));
  const anyFail = $derived(!!review && review.checks.some(([n, s]) => checkWord(s, receiptOf(n)).tone === "bad"));
  const needsApproval = $derived(!!review?.approval_token && !approved);
  const empty = $derived(!!review && review.files.length === 0);
  const unguarded = $derived(review?.unguarded ?? []);
  // Triage can judge dozens of commands; the last few say what kind of help it was.
  const JUDGMENTS = 3;
  const judgments = $derived((review?.judgments ?? []).slice(-JUDGMENTS));

  // Same rule as intent::is_memory_note: a file directly in .kitsu/memory/.
  const isNote = (p: string) => /^\.kitsu\/memory\/[^/]+\.md$/.test(p);

  function answers(j: Judgment): { question: string; answer: string }[] {
    return Object.entries(j.answers).map(([question, a]) => ({
      question,
      answer:
        a?.p_yes != null
          ? t("review.judgeP", { p: a.p_yes.toFixed(2) })
          : a?.choice != null
            ? a.choice
            : a?.score != null
              ? String(a.score)
              : t("review.judgeUnknown", { reason: a?.unknown ?? j.reason ?? "?" }),
    }));
  }

  // Accepting what no check looked at is allowed, but it's said out loud
  // first: the confirmation names every such path.
  let confirming = $state(false);
  let confirmClose = true;
  let confirmButton: HTMLButtonElement | undefined = $state();
  $effect(() => {
    if (confirming) confirmButton?.focus();
  });

  export async function accept(closeTask = true) {
    if (!review || busy || empty) return;
    if (needsApproval) {
      app.notify(t("review.approveFirst"), "info");
      return;
    }
    if (unguarded.length && !confirming) {
      confirming = true;
      confirmClose = closeTask;
      return;
    }
    if (confirming) closeTask = confirmClose;
    confirming = false;
    busy = "accept";
    try {
      result = await api.accept(run.id, closeTask, approved ? (review.approval_token ?? undefined) : undefined);
      if (result.result === "applied") {
        app.accepted++;
        app.notify(closeTask ? t("review.acceptedClosed") : t("review.accepted"), "ok", true);
      }
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

<div class="card review" data-tour="review">
  <div class="head">
    <!-- Takes over from the working fox when the run finishes in front of you. -->
    {#if fresh}<Fox state="perk" size={32} />{/if}
    <div class="summary">
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
      {#if review && !empty && unguarded.length}
        <div class="verdict tone-warn">{t("review.unknownVerdict", { n: unguarded.length })}</div>
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
        <button class="file press" onclick={() => app.go({ kind: "diff", run: run.id, path: f.path })}>
          <span class="mono path">{f.path}</span>
          {#if unguarded.includes(f.path)}<span class="pill dim unchecked" title={t("review.unknownTitle")}>{t("review.unchecked")}</span>{/if}
          {#if review.protected.includes(f.path)}<span class="pill warn">{isNote(f.path) ? t("review.note") : t("review.protected")}</span>{/if}
          <span class="n mono">{#if f.added === null}{t("review.binary")}{:else}<span class="tone-ok">+{f.added}</span> <span class="tone-bad">−{f.removed}</span>{/if}</span>
        </button>
      {/each}
    </div>

    {#if review.checks.length}
      <div class="checks">
        {#each review.checks as [name, s], i (name)}
          {@const rc = receiptOf(name)}
          {@const w = checkWord(s, rc)}
          <div class="check">
            <div class="check-row">
              <span class="mono name">{name}</span>
              <!-- Re-keyed on the word, so a result arriving (not run → passes) settles in visibly. -->
              {#key w.word}<span class="pill {w.tone} settle" style:--i={i}>{w.word}</span>{/key}
              <!-- A carried receipt already says it; don't say it twice. -->
              {#if w.hint && rc?.binding !== "carried"}<span class="hint">{w.hint}</span>{/if}
            </div>
            <!-- What the mark stands on: the command as it ran, its exit code, the tree. -->
            {#if rc}
              {@const cmd = receiptCommand(rc)}
              <div class="receipt mono" title={rc.command}>
                <span class="r-label">{t("receipt.label")}</span>
                <span class="r-cmd">{cmd.fingerprint ? `${t("receipt.fingerprint")} ${cmd.text}` : `$ ${cmd.text}`}</span>
                <span class="r-facts">{receiptLine(rc)}</span>
                <span class="r-bind {rc.binding}">{t(`receipt.${rc.binding}`)}</span>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}

    {#if unguarded.length}
      <div class="unknown" role="note">
        <div class="u-head"><span class="pill warn">? {t("review.unknown")}</span><span class="u-title">{t("review.unknownTitle")}</span></div>
        <ul class="u-paths mono">
          {#each unguarded as p (p)}<li>{p}</li>{/each}
        </ul>
        <div class="hint">{t("review.unknownBody")}</div>
      </div>
    {/if}

    <!-- Nothing below this line is evidence: the agent's word, and probabilities. -->
    {#if review.claim || judgments.length}
      <div class="judgment">
        <div class="j-title">{t("review.judgment")}</div>
        {#if review.claim}
          <blockquote class="claim">
            <div class="j-who">{t("review.agentSays", { agent: run.agent })} <span class="hint">· {t("review.agentSaysHint")}</span></div>
            <div class="prose">{@html render(review.claim)}</div>
          </blockquote>
        {/if}
        {#each judgments as j (j.id)}
          {#each answers(j) as a (a.question)}
            <div class="j-row">
              <span class="mono">{t("review.judged", { purpose: j.purpose, question: a.question, answer: a.answer })}</span>
              {#if j.model}<span class="hint">· {t("review.judgeBy", { model: j.model })}</span>{/if}
            </div>
          {/each}
        {/each}
        {#if review.judgments.length > JUDGMENTS}<div class="hint">+{i18n.number(review.judgments.length - JUDGMENTS)}</div>{/if}
      </div>
    {/if}

    {#if review.approval_token}
      <label class="approve">
        <input type="checkbox" bind:checked={approved} />
        <span>{t(review.protected.every(isNote) ? "review.approveNotes" : "review.approve", { paths: review.protected.join(", ") })} <strong>{t("review.approveStrong")}</strong></span>
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
  {:else if confirming && review}
    <!-- Esc cancels here instead of leaving the page. -->
    <div
      class="confirm"
      role="alertdialog"
      aria-labelledby="confirm-unguarded"
      tabindex="-1"
      onkeydown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          confirming = false;
        }
      }}
    >
      <p id="confirm-unguarded">{t("review.confirmUnguarded", { n: unguarded.length, paths: i18n.list(unguarded) })}</p>
      <div class="row">
        <button class="btn primary" bind:this={confirmButton} disabled={busy !== ""} onclick={() => accept(confirmClose)}>{t("review.acceptAnyway")} <kbd>a</kbd></button>
        <button class="btn quiet" onclick={() => (confirming = false)}>{t("review.cancel")} <kbd>Esc</kbd></button>
      </div>
    </div>
  {:else}
    <div class="actions" data-tour="review-actions">
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
    gap: var(--s5);
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--s3);
  }
  .summary {
    flex: 1;
    min-width: 0;
  }
  .title {
    font-weight: 600;
    font-size: 15px;
  }
  .verdict {
    margin-top: var(--s1);
    font-size: 13.5px;
    animation: fade-in var(--quick) var(--ease);
  }
  .files {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--line);
    border-radius: 10px;
    overflow: hidden;
  }
  .file {
    --press: 0.995;
    display: flex;
    align-items: center;
    gap: var(--s3);
    padding: 10px var(--s4);
    text-align: left;
    border-bottom: 1px solid var(--line);
  }
  .file:focus-visible {
    outline-offset: -2px;
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
    gap: var(--s3);
  }
  .check {
    display: flex;
    flex-direction: column;
    gap: var(--s1);
  }
  .check-row {
    display: flex;
    align-items: center;
    gap: var(--s3);
    font-size: 13px;
  }
  .check .name {
    min-width: 120px;
  }
  /* A receipt reads like one: monospace facts on a ruled slip, indented
     under the mark it backs. */
  .receipt {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    column-gap: var(--s3);
    row-gap: 2px;
    margin-left: calc(120px + var(--s3));
    padding: 6px var(--s3);
    border-left: 2px solid var(--line);
    background: var(--rail);
    border-radius: 0 6px 6px 0;
    font-size: 11.5px;
    color: var(--muted);
  }
  .r-label {
    color: var(--faint);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: 10.5px;
  }
  .r-cmd {
    color: var(--text);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .r-bind.current {
    color: var(--ok);
  }
  .r-bind.stale {
    color: var(--warn);
  }
  .unchecked::before {
    content: "?";
    font-weight: 700;
  }
  /* Unknown: not a failure, not a pass. A dashed edge, never a green one. */
  .unknown {
    display: flex;
    flex-direction: column;
    gap: var(--s2);
    padding: var(--s3) var(--s4);
    border: 1px dashed var(--warn);
    border-radius: 10px;
  }
  .u-head {
    display: flex;
    align-items: center;
    gap: var(--s3);
  }
  .u-title {
    font-weight: 600;
    font-size: 13px;
  }
  .u-paths {
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 12.5px;
  }
  /* Judgment: someone's word, set apart from receipts. Quoted, in italics,
     with no status color at all. */
  .judgment {
    display: flex;
    flex-direction: column;
    gap: var(--s2);
    padding-top: var(--s3);
    border-top: 1px dashed var(--line);
  }
  .j-title {
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--faint);
  }
  .claim {
    margin: 0;
    padding: 2px 0 2px var(--s3);
    border-left: 2px dotted var(--faint);
    font-style: italic;
    color: var(--muted);
  }
  .claim .prose {
    font-size: 13px;
  }
  .claim .prose :global(p) {
    margin: 0;
  }
  .j-who {
    font-style: normal;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }
  .j-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s2);
    font-size: 12px;
    color: var(--muted);
  }
  .confirm {
    padding: var(--s3) var(--s4);
    border-radius: 10px;
    background: var(--warn-soft);
    font-size: 13px;
    animation: enter var(--quick) var(--ease);
  }
  .confirm p {
    margin: 0 0 var(--s2);
  }
  .confirm:focus {
    outline: none;
  }
  /* The pill starts as a neutral chip and takes on its result's tone. */
  .settle {
    animation: settle 280ms var(--ease) backwards;
    animation-delay: calc(var(--i, 0) * 70ms);
  }
  @keyframes settle {
    from {
      opacity: 0.4;
      transform: scale(0.9);
      background-color: var(--hover);
      color: var(--muted);
    }
  }
  .approve {
    display: flex;
    gap: var(--s3);
    align-items: flex-start;
    padding: var(--s3) var(--s4);
    border-radius: 10px;
    background: var(--warn-soft);
    font-size: 13px;
  }
  .approve input {
    margin-top: 3px;
  }
  .outcome {
    padding: var(--s3) var(--s4);
    border-radius: 10px;
    background: var(--rail);
    font-size: 13px;
    animation: enter var(--quick) var(--ease);
  }
  .actions,
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s2);
  }
  .actions {
    padding-top: var(--s1);
  }
  /* Keeps Discard at the right edge even when a narrow window wraps the row. */
  .actions > .danger {
    margin-left: auto;
  }
  .spacer {
    flex: 1;
  }
  .continue {
    display: flex;
    flex-direction: column;
    gap: var(--s2);
    animation: enter var(--quick) var(--ease);
  }
  .activity {
    animation: enter var(--quick) var(--ease);
    padding: var(--s4);
    border-radius: 10px;
    background: var(--bg);
    border: 1px solid var(--line);
  }
</style>
