<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { api, errorText } from "../lib/api";
  import { ago, duration } from "../lib/format";
  import { render } from "../lib/md";
  import type { Evidence, Run, RunEvent } from "../lib/types";

  let { runId, live = false, compact = false }: { runId: string; live?: boolean; compact?: boolean } = $props();

  let run = $state<Run | null>(null);
  let events = $state<RunEvent[]>([]);
  let evidence = $state<Evidence[]>([]);
  let error = $state<string | null>(null);
  let lastSeq = 0;
  let loadedFor = "";
  let box: HTMLDivElement | undefined = $state();

  async function load(reset: boolean) {
    if (reset) {
      lastSeq = 0;
      events = [];
    }
    try {
      const d = await api.run(runId, lastSeq);
      run = d.run;
      evidence = d.evidence;
      if (d.events.length) {
        events = [...events, ...d.events];
        lastSeq = d.events[d.events.length - 1]!.seq;
        if (live) queueMicrotask(() => box?.scrollTo({ top: box.scrollHeight, behavior: "smooth" }));
      }
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  $effect(() => {
    const id = runId;
    void app.tick;
    const reset = id !== loadedFor;
    loadedFor = id;
    void load(reset);
  });

  // Tool calls arrive as a start and then updates; show each call once, in
  // its latest state, at the position it started.
  type Row =
    | { type: "message"; key: string; text: string }
    | { type: "tool"; key: string; title: string; status: string; kind: string; files: string[] }
    | { type: "plan"; key: string; entries: { content: string; status: string }[] }
    | { type: "note"; key: string; text: string; tone: string };

  const rows = $derived.by(() => {
    const out: Row[] = [];
    const tools = new Map<string, Extract<Row, { type: "tool" }>>();
    let plan: Extract<Row, { type: "plan" }> | null = null;
    for (const e of events) {
      const b = e.body;
      switch (e.kind) {
        case "agent.message":
          out.push({ type: "message", key: `m${e.seq}`, text: String(b.text ?? "") });
          break;
        case "agent.tool": {
          const existing = tools.get(b.id);
          if (existing) {
            existing.status = b.status ?? existing.status;
            existing.title = b.title ?? existing.title;
          } else {
            const row = { type: "tool" as const, key: `t${e.seq}`, title: b.title ?? "tool", status: b.status ?? "pending", kind: b.kind ?? "other", files: b.locations ?? [] };
            tools.set(b.id, row);
            out.push(row);
          }
          break;
        }
        case "agent.plan":
          if (plan) plan.entries = b.entries;
          else {
            plan = { type: "plan", key: `p${e.seq}`, entries: b.entries };
            out.push(plan);
          }
          break;
        case "permission":
          out.push({ type: "note", key: `n${e.seq}`, text: `allowed “${b.title}” (${b.by})`, tone: "dim" });
          break;
        case "ask.open":
          out.push({ type: "note", key: `n${e.seq}`, text: `asked you: ${b.request?.title}`, tone: "warn" });
          break;
        case "ask.answered":
          out.push({ type: "note", key: `n${e.seq}`, text: `you answered: ${b.answer}`, tone: "dim" });
          break;
        case "check.done":
          out.push({ type: "note", key: `n${e.seq}`, text: `check ${b.check}: ${b.outcome}`, tone: b.outcome === "pass" ? "ok" : "bad" });
          break;
        case "protocol.duplicate_response":
        case "protocol.violation":
          out.push({ type: "note", key: `n${e.seq}`, text: e.kind === "protocol.violation" ? `agent broke the protocol: ${b.detail}` : "agent answered the same request twice (ignored)", tone: "bad" });
          break;
        case "run.state":
          if (b.to === "stopping") out.push({ type: "note", key: `n${e.seq}`, text: "stop requested", tone: "dim" });
          if (b.to === "failed") out.push({ type: "note", key: `n${e.seq}`, text: `failed: ${b.detail ?? ""}`, tone: "bad" });
          if (b.to === "interrupted") out.push({ type: "note", key: `n${e.seq}`, text: "interrupted: the process that owned this run went away", tone: "warn" });
          break;
      }
    }
    return compact ? out.slice(-6) : out;
  });

  const toolIcon: Record<string, string> = { read: "◇", search: "⌕", edit: "✎", delete: "✕", move: "↦", execute: "›_", fetch: "⇣", think: "…", other: "•" };
</script>

<div class="activity" class:compact>
  {#if error}<div class="err">{error}</div>{/if}
  {#if run}
    <div class="meta hint">
      {run.agent} · started {ago(run.created_at)}{#if run.ended_at} · took {duration(run.ended_at - run.created_at)}{/if}{#if run.note} · note: “{run.note}”{/if}
    </div>
  {/if}
  <div class="rows scroll" bind:this={box}>
    {#each rows as r (r.key)}
      {#if r.type === "message"}
        <div class="msg prose">{@html render(r.text)}</div>
      {:else if r.type === "tool"}
        <div class="tool {r.status}">
          <span class="icon mono">{toolIcon[r.kind] ?? "•"}</span>
          <span class="t">{r.title}</span>
          <span class="st">{r.status === "completed" ? "" : r.status.replace("_", " ")}</span>
        </div>
      {:else if r.type === "plan"}
        <ul class="plan">
          {#each r.entries as p, i (i)}
            <li class={p.status}><span class="box"></span>{p.content}</li>
          {/each}
        </ul>
      {:else}
        <div class="note tone-{r.tone}">{r.text}</div>
      {/if}
    {/each}
    {#if live && run && !run.ended_at}
      <div class="alive"><span></span><span></span><span></span></div>
    {/if}
    {#if !rows.length && !live}<div class="hint">No activity recorded.</div>{/if}
  </div>
  {#if evidence.length && !compact}
    <div class="checks">
      {#each evidence as e (e.id)}
        <span class="pill {e.outcome === 'pass' ? 'ok' : 'bad'}" title={e.command}>{e.check_name} {e.outcome === "pass" ? "passes" : e.outcome} · {duration(e.duration_ms)}</span>
      {/each}
    </div>
  {/if}
</div>

<style>
  .activity {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 420px;
  }
  .compact .rows {
    max-height: 260px;
  }
  .msg {
    color: var(--text);
  }
  .msg :global(p) {
    margin: 0 0 4px;
  }
  .tool {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 10px;
    border-radius: 8px;
    background: var(--rail);
    font-size: 13px;
  }
  .tool .icon {
    width: 18px;
    color: var(--faint);
    text-align: center;
  }
  .tool .t {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tool .st {
    color: var(--muted);
    font-size: 12px;
  }
  .tool.failed .st,
  .tool.failed .t {
    color: var(--bad);
  }
  .tool.in_progress .st {
    color: var(--work);
  }
  .plan {
    margin: 2px 0;
    padding: 0;
    list-style: none;
    font-size: 13px;
  }
  .plan li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 0;
    color: var(--muted);
  }
  .plan .box {
    width: 12px;
    height: 12px;
    border: 1.5px solid var(--faint);
    border-radius: 4px;
    flex: none;
  }
  .plan .completed {
    color: var(--faint);
    text-decoration: line-through;
  }
  .plan .completed .box {
    background: var(--faint);
    border-color: var(--faint);
  }
  .plan .in_progress {
    color: var(--text);
  }
  .plan .in_progress .box {
    border-color: var(--work);
  }
  .note {
    font-size: 12.5px;
  }
  .meta {
    margin-bottom: 2px;
  }
  .checks {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .alive {
    display: flex;
    gap: 4px;
    padding: 6px 2px;
  }
  .alive span {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--work);
    animation: pulse 1.2s infinite ease-in-out;
  }
  .alive span:nth-child(2) {
    animation-delay: 0.15s;
  }
  .alive span:nth-child(3) {
    animation-delay: 0.3s;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 0.25;
    }
    50% {
      opacity: 1;
    }
  }
  .err {
    color: var(--bad);
    font-size: 13px;
  }
</style>
