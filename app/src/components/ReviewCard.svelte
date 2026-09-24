<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { api, errorKind, errorText } from "../lib/api";
  import { checkWord } from "../lib/format";
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
      app.notify("This change touches protected files. Look at them, then tick “I reviewed these”.", "info");
      return;
    }
    busy = "accept";
    try {
      result = await api.accept(run.id, closeTask, approved ? (review.approval_token ?? undefined) : undefined);
      if (result.result === "applied") app.notify(closeTask ? "Accepted. Task closed." : "Accepted.", "ok");
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
      app.notify("Discarded.");
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
      app.notify(`Continuing with ${app.prefs.agent}.`);
    } catch (e) {
      app.notify(errorKind(e) === "denied" ? errorText(e) : `Could not start: ${errorText(e)}`, "bad");
    } finally {
      busy = "";
    }
  }
</script>

<div class="card review">
  <div class="head">
    <div>
      <div class="title">
        {run.agent} {run.stop_reason === "cancelled" ? "was stopped" : "finished"}
        {#if review}<span class="hint">· {review.files.length} file{review.files.length === 1 ? "" : "s"} · <span class="tone-ok">+{total[0]}</span> <span class="tone-bad">−{total[1]}</span> · onto <span class="mono">{review.target}</span></span>{/if}
      </div>
      {#if empty}
        <div class="verdict hint">It changed nothing. Nothing to accept; continue with a note or discard it.</div>
      {:else if review && review.checks.length}
        <div class="verdict {allPass ? 'tone-ok' : anyFail ? 'tone-bad' : 'tone-warn'}">
          {allPass ? "Every required check passes on this change." : anyFail ? "A required check fails. The agent's own summary doesn't count." : "Some required checks haven't run on this change yet."}
        </div>
      {:else if review}
        <div class="verdict hint">No checks apply to this change. You're the only judge here.</div>
      {/if}
    </div>
    <button class="btn quiet" onclick={() => (showActivity = !showActivity)}>{showActivity ? "Hide" : "What it did"}</button>
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
          {#if review.protected.includes(f.path)}<span class="pill warn">rules / protected</span>{/if}
          <span class="n mono">{#if f.added === null}binary{:else}<span class="tone-ok">+{f.added}</span> <span class="tone-bad">−{f.removed}</span>{/if}</span>
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
        <span
          >This change edits rules or protected files ({review.protected.join(", ")}). Checks are judged by the rules in <em>your</em> checkout,
          not the agent's copy, but look at these yourself. <strong>I reviewed these.</strong></span
        >
      </label>
    {/if}
  {/if}

  {#if result && result.result !== "applied"}
    <div class="outcome">
      {#if result.result === "conflict"}
        <strong class="tone-bad">Conflicts with {review?.target}</strong> in {result.paths.join(", ")}. Continue on top of the current branch and let the agent resolve it.
      {:else if result.result === "checks_failed"}
        <strong class="tone-bad">Checks fail on the combined result</strong> ({result.failing.join(", ")}). It passed in its own worktree, but not together with what's on {review?.target} now.
      {:else if result.result === "needs_approval"}
        <strong class="tone-warn">Needs your approval</strong> for {result.paths.join(", ")}.
      {/if}
    </div>
  {/if}

  {#if noting}
    <div class="continue">
      <textarea class="field" rows="2" placeholder="What should the next attempt do differently? (optional)" bind:value={note}></textarea>
      <div class="row">
        <button class="btn primary" disabled={busy !== ""} onclick={startContinue}>Continue with {app.prefs.agent}</button>
        <button class="btn quiet" onclick={() => (noting = false)}>Cancel</button>
      </div>
    </div>
  {:else}
    <div class="actions">
      {#if !empty}
        <button class="btn primary" disabled={!review || busy !== "" || needsApproval} onclick={() => accept(true)}>Accept and close task <kbd>a</kbd></button>
        <button class="btn" disabled={!review || busy !== "" || needsApproval} onclick={() => accept(false)}>Accept, keep open</button>
      {/if}
      <button class="btn" disabled={busy !== ""} onclick={continueWork}>Continue… <kbd>c</kbd></button>
      <span class="spacer"></span>
      <button class="btn quiet danger" disabled={busy !== ""} onclick={discard}>Discard <kbd>x</kbd></button>
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
