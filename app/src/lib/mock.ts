// Fixture data for running the UI in a plain browser: `npm run dev`, and the
// Playwright checks in tests/. It plays a small, realistic day on the
// retry-storm fixture so every state of the UI can be seen and clicked.
// Nothing here is used by the desktop app.

import type {
  Accepted,
  Ask,
  CheckStatus,
  Evidence,
  Overview,
  Rules,
  Run,
  RunDetail,
  RunEvent,
  TaskDetail,
  TaskView,
  Review,
} from "./types";

const listeners = new Set<() => void>();
const changed = () => setTimeout(() => listeners.forEach((l) => l()), 30);

export async function listen(_event: string, cb: () => void) {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

let seq = 100;
const now = () => Date.now();
const minutes = (m: number) => now() - m * 60_000;

interface MockTask {
  id: string;
  title: string;
  state: "open" | "done" | "dropped";
  scope: string[];
  checks: string[];
  after: string[];
  body: string;
}

const tasks: MockTask[] = [
  {
    id: "bounded-retries",
    title: "Stop retrying forever when the upstream times out",
    state: "open",
    scope: ["payments.py"],
    checks: ["retries"],
    after: [],
    body: "Last Tuesday the upstream browned out for 20 minutes and every web worker sat in `charge()` spinning on timeouts. That made our outage longer than theirs.",
  },
  {
    id: "refund-endpoint",
    title: "Add refunds that can't be issued twice",
    state: "open",
    scope: ["refunds.py", "payments.py"],
    checks: ["refunds"],
    after: [],
    body: "Support asked for partial refunds. Same rules as charges: a lost response must not turn into two refunds.",
  },
  {
    id: "metrics-export",
    title: "Export retry and timeout counts to the metrics endpoint",
    state: "open",
    scope: ["metrics.py"],
    checks: [],
    after: [],
    body: "We had no idea how often we were retrying until the outage.",
  },
  {
    id: "webhook-signatures",
    title: "Verify webhook signatures from the upstream",
    state: "open",
    scope: ["webhooks.py"],
    checks: ["webhooks"],
    after: [],
    body: "Right now anybody can POST a fake 'charge.succeeded'.",
  },
  {
    id: "dashboard-latency",
    title: "Show upstream latency on the ops dashboard",
    state: "open",
    scope: ["ops/**"],
    checks: [],
    after: ["metrics-export"],
    body: "Needs the metrics export first.",
  },
  {
    id: "remove-legacy-client",
    title: "Delete the legacy SOAP client",
    state: "done",
    scope: ["legacy/**"],
    checks: [],
    after: [],
    body: "",
  },
];

const questions = [
  {
    id: "webhook-secret-rotation",
    title: "Do we have to accept both old and new webhook secrets during rotation?",
    open: true,
    blocks: ["webhook-signatures"],
    answer: null as string | null,
    body: "If yes, verification needs a list of secrets and a way to retire one.",
  },
];

const runs: Run[] = [];
const events: RunEvent[] = [];
const asks: Ask[] = [];
const evidence: Evidence[] = [];

function run(partial: Partial<Run> & { id: string; task: string }): Run {
  const r: Run = {
    agent: "claude",
    base: "71dce3e6eed1e056367382bffa8a9aa4a5470d5c",
    branch: `kitsu/run/${partial.id}`,
    worktree: `/home/you/payments/.git/kitsu/worktrees/${partial.id}`,
    state: "running",
    stop_reason: null,
    detail: null,
    cancel_requested: false,
    snapshot: null,
    resolution: null,
    usage: null,
    from_run: null,
    note: null,
    created_at: minutes(12),
    ended_at: null,
    changed: null,
    ...partial,
  };
  runs.push(r);
  return r;
}

function ev(runId: string, kind: string, body: unknown, at = now()) {
  events.push({ seq: ++seq, run: runId, at, kind, body });
}

// A finished, verified attempt waiting for review.
const good = run({ id: "r7k2mq", task: "bounded-retries", state: "finished", stop_reason: "end_turn", snapshot: "c0ffee1", ended_at: minutes(3), created_at: minutes(9), changed: ["payments.py"], usage: { input: 18400, output: 2900, cached_read: 61000, total: 82300, context_used: 41000, context_size: 200000, cost: 0.19, currency: "USD" } });
ev(good.id, "run.state", { to: "running" }, minutes(9));
ev(good.id, "agent.plan", { entries: [{ content: "Bound the retry loop to 3 attempts", status: "completed" }, { content: "Create the idempotency key once, before the first attempt", status: "completed" }, { content: "Run the checks", status: "completed" }] }, minutes(8));
ev(good.id, "agent.message", { text: "Reading the charge path and the retry loop first." }, minutes(8));
ev(good.id, "agent.tool", { id: "t1", title: "Read payments.py", kind: "read", status: "completed", locations: ["payments.py"] }, minutes(8));
ev(good.id, "agent.tool", { id: "t2", title: "Edit payments.py", kind: "edit", status: "completed", locations: ["payments.py"] }, minutes(6));
ev(good.id, "agent.message", { text: "Bounded to 3 attempts with jittered backoff; the key is created once per charge so a lost response can't double-charge." }, minutes(4));
ev(good.id, "run.state", { to: "finished", stop_reason: "end_turn" }, minutes(3));
for (const [name, cmd] of [["retries", "python3 -m unittest -q test_retries"], ["idempotency", "python3 -m unittest -q test_idempotency"]] as const) {
  evidence.push({ id: evidence.length + 1, check_name: name, command: cmd, tree: "t-good", tree_after: null, outcome: "pass", exit_code: 0, duration_ms: 140, run: good.id, started_at: minutes(3) });
  ev(good.id, "check.done", { check: name, outcome: "pass" }, minutes(3));
}

// An agent currently working, streaming as you watch.
const live = run({ id: "r9fh3p", task: "refund-endpoint", agent: "codex", created_at: minutes(2) });
ev(live.id, "run.state", { to: "running" }, minutes(2));
ev(live.id, "agent.plan", { entries: [{ content: "Add refund() next to charge()", status: "in_progress" }, { content: "Reuse the idempotency key rule", status: "pending" }, { content: "Tests for partial refunds", status: "pending" }] }, minutes(2));
ev(live.id, "agent.tool", { id: "t1", title: "Read payments.py", kind: "read", status: "completed", locations: ["payments.py"] }, minutes(1.5));
ev(live.id, "agent.message", { text: "The charge path already creates one key per logical operation. Refunds need the same, keyed by (charge id, refund request id)." }, minutes(1));

// One waiting on you.
const asking = run({ id: "r3xw8d", task: "metrics-export", agent: "claude", created_at: minutes(1) });
ev(asking.id, "run.state", { to: "running" }, minutes(1));
ev(asking.id, "agent.tool", { id: "t1", title: "Edit metrics.py", kind: "edit", status: "completed", locations: ["metrics.py"] }, minutes(0.6));
asks.push({ id: 41, run: asking.id, request: { title: "Run `pip install prometheus-client`", kind: "execute", options: [{ optionId: "allow", name: "Allow once", kind: "allow_once" }, { optionId: "deny", name: "Deny", kind: "reject_once" }] }, answer: null, created_at: minutes(0.4) });
ev(asking.id, "ask.open", { ask: 41, request: asks[0]!.request }, minutes(0.4));

const liveScript = [
  { kind: "agent.tool", body: { id: "t2", title: "Edit payments.py", kind: "edit", status: "in_progress", locations: ["payments.py"] } },
  { kind: "agent.tool", body: { id: "t2", title: "Edit payments.py", kind: "edit", status: "completed", locations: ["payments.py"] } },
  { kind: "agent.message", body: { text: "Added refund() with the key created before the first attempt. Writing tests for the lost-response case." } },
  { kind: "agent.tool", body: { id: "t3", title: "Write test_refunds.py", kind: "edit", status: "completed", locations: ["test_refunds.py"] } },
];
let step = 0;
setInterval(() => {
  if (live.state !== "running" || step >= liveScript.length) return;
  const s = liveScript[step++]!;
  ev(live.id, s.kind, s.body);
  changed();
}, 2500);

const checkStatus = (name: string, runId?: string): CheckStatus => {
  const e = [...evidence].reverse().find((x) => x.check_name === name && (!runId || x.run === runId));
  if (!e) return { status: "unverified" };
  return { status: "current", outcome: e.outcome, evidence: e.id };
};

function view(t: MockTask): TaskView {
  const others = Math.max(0, runs.filter((r) => r.task === t.id && !r.resolution).length - 1);
  const base = { id: t.id, title: t.title, path: `.kitsu/tasks/${t.id}.md`, others };
  if (t.state === "done") return { ...base, status: { kind: "done" }, attention: "quiet", reason: "done" };
  const open = runs.filter((r) => r.task === t.id && !r.resolution);
  const liveRun = open.find((r) => ["starting", "running", "stopping"].includes(r.state));
  if (liveRun) {
    const n = asks.filter((a) => a.run === liveRun.id && !a.answer).length;
    if (n) return { ...base, status: { kind: "asking", run: liveRun.id, agent: liveRun.agent, asks: n }, attention: "needs_you", reason: `${liveRun.agent} is waiting on a question from you` };
    return { ...base, status: { kind: "running", run: liveRun.id, agent: liveRun.agent, stopping: liveRun.state === "stopping" }, attention: "working", reason: `${liveRun.agent} is working on it` };
  }
  const fin = open.find((r) => r.state === "finished");
  if (fin) {
    const failing = t.checks.concat(t.id === "bounded-retries" ? ["idempotency"] : []).filter((c) => checkStatus(c, fin.id).status !== "current");
    return { ...base, status: { kind: "review", run: fin.id, agent: fin.agent, verdict: failing.length ? "unverified" : "verified", failing: [] }, attention: "needs_you", reason: failing.length ? "ready for review, not verified yet" : "ready for review, checks pass" };
  }
  const failed = open[0];
  if (failed) return { ...base, status: { kind: "failed", run: failed.id, agent: failed.agent, detail: failed.detail ?? "failed" }, attention: "needs_you", reason: `${failed.agent} failed: ${failed.detail}` };
  const q = questions.filter((q) => q.open && q.blocks.includes(t.id)).map((q) => q.id);
  if (q.length) return { ...base, status: { kind: "blocked_by_question", questions: q }, attention: "needs_you", reason: `blocked on ${q.join(", ")}` };
  const deps = t.after.filter((d) => tasks.find((x) => x.id === d)?.state === "open");
  if (deps.length) return { ...base, status: { kind: "blocked_by_tasks", tasks: deps }, attention: "waiting", reason: `waiting on ${deps.join(", ")}` };
  return { ...base, status: { kind: "ready" }, attention: "ready", reason: "ready to start" };
}

const order = ["needs_you", "working", "ready", "waiting", "quiet"];

let seen = 0;

const handlers: Record<string, (a: Record<string, unknown>) => unknown> = {
  open_repo: () => ({ root: "/home/you/payments", name: "payments", branch: "main", trusted: true, initialized: true }),
  trust_repo: () => handlers.open_repo!({}),
  init_repo: () => handlers.open_repo!({}),
  overview: (): Overview => ({
    repo: handlers.open_repo!({}) as Overview["repo"],
    tasks: tasks.map(view).sort((a, b) => order.indexOf(a.attention) - order.indexOf(b.attention) || a.id.localeCompare(b.id)),
    asks: asks.filter((a) => !a.answer),
    since: {
      from_seq: seen,
      to_seq: seq,
      items: seen >= seq ? [] : [
        { kind: "ask", run: asking.id, task: "metrics-export", text: "the agent asks: Run `pip install prometheus-client`", params: { title: "Run `pip install prometheus-client`" } },
        { kind: "finished", run: good.id, task: "bounded-retries", text: "finished, ready for review", params: { stop_reason: "end_turn" } },
      ],
    },
    agents: [
      { name: "claude", command: "npx -y @agentclientprotocol/claude-agent-acp", source: "preset" },
      { name: "codex", command: "npx -y @agentclientprotocol/codex-acp", source: "preset" },
      { name: "gemini", command: "gemini --acp", source: "preset" },
      { name: "opencode", command: "opencode acp", source: "preset" },
      { name: "test", command: "kitsu-test-agent", source: "bundled" },
    ],
    problems: [],
    counts: { decisions: 2, memory: 3, open_questions: questions.filter((q) => q.open).length, checks: 3 },
  }),
  mark_seen: (a) => {
    seen = a.seq as number;
  },
  task_detail: (a): TaskDetail => {
    const t = tasks.find((x) => x.id === a.id)!;
    return {
      task: { ...t, path: `.kitsu/tasks/${t.id}.md` },
      brief: {
        task: t.id,
        markdown: `# ${t.title}\n\n${t.body}\n\n## Done means\n- Check \`retries\` passes\n\n## Must hold\n- **One idempotency key per logical charge, reused by every retry**`,
        included:
          t.id === "webhook-signatures"
            ? [{ kind: "task", id: t.id, title: t.title, path: `.kitsu/tasks/${t.id}.md`, content_id: "a1", why: "the task" }]
            : [
                { kind: "task", id: t.id, title: t.title, path: `.kitsu/tasks/${t.id}.md`, content_id: "a1", why: "the task" },
                { kind: "decision", id: "retry-budget", title: "Retry charges at most 3 times, only with an idempotency key", path: ".kitsu/decisions/retry-budget.md", content_id: "c3", why: "task scope payments.py overlaps payments.py" },
              ],
        omitted: [],
        problems: [],
      },
      runs: runs.filter((r) => r.task === t.id).sort((x, y) => y.created_at - x.created_at),
      questions: questions.filter((q) => q.blocks.includes(t.id)),
      dirty_checkout: true,
    };
  },
  run_detail: (a): RunDetail => ({
    run: runs.find((r) => r.id === a.id)!,
    events: events.filter((e) => e.run === a.id && e.seq > (a.after as number)),
    evidence: evidence.filter((e) => e.run === a.id),
  }),
  evidence_log: () => "....\n----------------------------------------------------------------------\nRan 2 tests in 0.001s\n\nOK\n",
  review: (a): Review => {
    const r = runs.find((x) => x.id === a.run)!;
    const t = tasks.find((x) => x.id === r.task)!;
    const checks = [...new Set(t.checks.concat(t.id === "bounded-retries" ? ["idempotency"] : []))];
    return {
      run: r.id,
      target: "main",
      target_head: "71dce3e",
      from: "71dce3e",
      files: t.id === "bounded-retries" ? [{ path: "payments.py", added: 18, removed: 6 }] : [{ path: "refunds.py", added: 64, removed: 0 }, { path: "test_refunds.py", added: 41, removed: 0 }],
      protected: [],
      approval_token: null,
      checks: checks.map((c) => [c, checkStatus(c, r.id)] as [string, CheckStatus]),
    };
  },
  file_diff: (a) => ({
    path: a.path,
    old: '"""Charges a card through an upstream payment API."""\n\nimport uuid\n\n\nclass Timeout(Exception):\n    pass\n\n\ndef charge(upstream, card, amount):\n    # Upstream has been flaky, so keep trying until it goes through.\n    while True:\n        try:\n            return upstream.post("/charges", card=card, amount=amount)\n        except Timeout:\n            continue\n',
    new: '"""Charges a card through an upstream payment API."""\n\nimport random\nimport time\nimport uuid\n\n\nclass Timeout(Exception):\n    pass\n\n\nMAX_ATTEMPTS = 3\n\n\ndef charge(upstream, card, amount, sleep=time.sleep):\n    # One key for the whole logical charge. If the upstream charged us and\n    # the response got lost, the retry carries the same key and is a no-op.\n    key = str(uuid.uuid4())\n    for attempt in range(MAX_ATTEMPTS):\n        try:\n            return upstream.post("/charges", card=card, amount=amount, idempotency_key=key)\n        except Timeout:\n            if attempt == MAX_ATTEMPTS - 1:\n                raise\n            sleep(random.uniform(0, 0.05 * 2**attempt))\n',
    protected: false,
  }),
  start_run: (a) => {
    const id = "r" + Math.random().toString(36).slice(2, 8);
    const r = run({ id, task: a.task as string, agent: a.agent as string, created_at: now(), note: (a.note as string) ?? null });
    ev(id, "run.state", { to: "running" });
    let i = 0;
    const script = [
      { kind: "agent.message", body: { text: "Reading the relevant files." } },
      { kind: "agent.tool", body: { id: "t1", title: "Read metrics.py", kind: "read", status: "completed", locations: ["metrics.py"] } },
      { kind: "agent.tool", body: { id: "t2", title: "Edit metrics.py", kind: "edit", status: "completed", locations: ["metrics.py"] } },
      { kind: "agent.message", body: { text: "Done. Counters are exported under payments_upstream_*." } },
    ];
    const timer = setInterval(() => {
      if (r.state !== "running") return clearInterval(timer);
      const s = script[i++];
      if (!s) {
        r.state = "finished";
        r.stop_reason = "end_turn";
        r.snapshot = "abc1234";
        r.ended_at = now();
        r.changed = ["metrics.py"];
        ev(id, "run.state", { to: "finished", stop_reason: "end_turn" });
        clearInterval(timer);
      } else ev(id, s.kind, s.body);
      changed();
    }, 1200);
    changed();
    return id;
  },
  stop_run: (a) => {
    const r = runs.find((x) => x.id === a.id)!;
    r.state = "finished";
    r.stop_reason = "cancelled";
    r.cancel_requested = true;
    r.snapshot = "partial1";
    r.changed = ["refunds.py"];
    ev(r.id, "run.state", { to: "finished", stop_reason: "cancelled" });
    changed();
  },
  answer_ask: (a) => {
    const ask = asks.find((x) => x.id === a.id)!;
    ask.answer = a.option as string;
    ev(ask.run, "ask.answered", { ask: ask.id, answer: ask.answer });
    changed();
    return true;
  },
  answer_question: (a) => {
    const q = questions.find((x) => x.id === a.id)!;
    q.open = false;
    q.answer = a.answer as string;
    changed();
  },
  accept_run: (a): Accepted => {
    const r = runs.find((x) => x.id === a.run)!;
    r.resolution = "accepted";
    const t = tasks.find((x) => x.id === r.task)!;
    if (a.closeTask) t.state = "done";
    ev(r.id, "integration.state", { state: "applied" });
    changed();
    return { result: "applied", commit: "5231cde353aa", closed_task: !!a.closeTask, notes: [] };
  },
  discard_run: (a) => {
    runs.find((x) => x.id === a.run)!.resolution = "discarded";
    changed();
    return [];
  },
  new_entity: (a) => {
    const id = String(a.title).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 48);
    if (a.kind === "task") tasks.push({ id, title: a.title as string, state: "open", scope: (a.scope as string[]) ?? [], checks: (a.checks as string[]) ?? [], after: (a.after as string[]) ?? [], body: (a.body as string) ?? "" });
    changed();
    return { id, path: `.kitsu/tasks/${id}.md` };
  },
  rules: (): Rules => ({
    decisions: [
      { id: "one-key-per-charge", title: "One idempotency key per logical charge, reused by every retry", state: "accepted", scope: ["payments.py"], rejected: [], body: "The upstream can charge the card and then lose the response. The only thing that makes a retry safe is sending the same `idempotency_key` again.", path: ".kitsu/decisions/one-key-per-charge.md" },
      { id: "retry-budget", title: "Retry charges at most 3 times, only with an idempotency key", state: "accepted", scope: ["payments.py"], rejected: ["Infinite retry: turns an upstream brownout into our outage", "Circuit breaker for now: one caller, no evidence of long outages", "Retrying without a key: a lost response becomes a second charge"], body: "Three attempts total, with jittered backoff between them.", path: ".kitsu/decisions/retry-budget.md" },
    ],
    questions: questions.map((q) => ({ ...q, path: `.kitsu/questions/${q.id}.md` })),
    memory: [
      { id: "upstream-dedupes", title: "The upstream dedupes idempotency keys for 24 hours", kind: "fact", scope: ["payments.py"], anchors: ["payments.py"], by: "you", run: null, body: "From their API docs, section Idempotency. A retry after 24h is a new charge.", path: ".kitsu/memory/upstream-dedupes.md", freshness: { status: "current" }, personal: false, state: "current", superseded_by: null, reason: null, key: null },
      { id: "fake-upstream-sleeps", title: "fake_upstream sleeps for real unless you pass sleep=", kind: "gotcha", scope: ["test_*.py"], anchors: ["fake_upstream.py"], by: "claude", run: "r7k2mq", body: "Tests take 20s instead of 0.2s if you forget it.", path: ".kitsu/memory/fake-upstream-sleeps.md", freshness: { status: "stale", changed: ["fake_upstream.py"], since: "71dce3e" }, personal: false, state: "current", superseded_by: null, reason: null, key: null },
      { id: "personal/small-commits", title: "Small commits, one idea each", kind: "preference", scope: [], anchors: [], by: null, run: null, body: "", path: "~/.config/kitsu/memory/small-commits.md", freshness: { status: "unanchored" }, personal: true, state: "current", superseded_by: null, reason: null, key: null },
    ],
    checks: [
      { name: "retries", run: "python3 -m unittest -q test_retries", scope: [], guards: [], why: null, status: { status: "stale", outcome: "fail", evidence: 1, changed: ["payments.py"], more: 0 } },
      { name: "idempotency", run: "python3 -m unittest -q test_idempotency", scope: ["payments.py"], guards: ["payments.py"], why: "One idempotency key per logical charge, reused by every retry", status: { status: "stale", outcome: "fail", evidence: 2, changed: ["payments.py"], more: 0 } },
      { name: "refunds", run: "python3 -m unittest -q test_refunds", scope: [], guards: [], why: null, status: { status: "unverified" } },
    ],
  }),
  run_checks: () => [],
  read_file: (a) => ({ path: a.path, text: (handlers.file_diff!(a) as { new: string }).new, version: "v1" }),
  write_file: () => "v2",
  list_files: () => ["payments.py", "fake_upstream.py", "test_idempotency.py", "test_retries.py", "metrics.py", ".kitsu/kitsu.toml", ".kitsu/tasks/bounded-retries.md", ".kitsu/decisions/one-key-per-charge.md", ".kitsu/decisions/retry-budget.md"],
};

export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const h = handlers[cmd];
  if (!h) throw { kind: "invalid", message: `mock: no handler for ${cmd}` };
  await new Promise((r) => setTimeout(r, 15));
  return structuredClone(h(args)) as T;
}
