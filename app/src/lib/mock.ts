// Fixture data for running the UI in a plain browser: `npm run dev`, and the
// Playwright checks in tests/. It plays a small, realistic day across three
// projects (payments, web, infra) so every state of the UI can be seen and
// clicked, switching included. Nothing here is used by the desktop app.
//
// Like the real backend, every command about a project names it: each
// project is its own little engine, and a command without a known `repo`
// is refused instead of landing in whichever one was open last.

import type {
  Accepted,
  Ask,
  BranchRow,
  CheckStatus,
  Evidence,
  Judgment,
  Receipt,
  Overview,
  Plan,
  Project,
  ProjectSummary,
  RepoTree,
  Rules,
  Run,
  RunDetail,
  RunEvent,
  TaskDetail,
  TaskView,
  Review,
  WorktreeRow,
} from "./types";

type Payload = { repo: string } | null;
const listeners = new Set<(p: Payload) => void>();
const changedIn = (repo: string) => setTimeout(() => listeners.forEach((l) => l({ repo })), 30);

export async function listen(_event: string, cb: (p: unknown) => void) {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

let seq = 100;
const now = () => Date.now();
const minutes = (m: number) => now() - m * 60_000;
const days = (d: number) => Math.round((now() - d * 86_400_000) / 1000);

interface MockTask {
  id: string;
  title: string;
  state: "open" | "done" | "dropped";
  scope: string[];
  checks: string[];
  after: string[];
  body: string;
}

interface MockQuestion {
  id: string;
  title: string;
  open: boolean;
  blocks: string[];
  answer: string | null;
  body: string;
}

type Handler = (a: Record<string, unknown>) => unknown;

/** What makes one project different from another; the engine is shared. */
interface Seed {
  root: string;
  name: string;
  branch: string;
  trusted: boolean;
  initialized?: boolean;
  tasks: MockTask[];
  questions: MockQuestion[];
  checks: string[];
  /** Checks that guard a path: required for any task whose scope has it. */
  guards?: Record<string, string>;
  /** What the files say; `enforced_by` and receipts are derived, like the backend does. */
  rules: { decisions: Omit<Rules["decisions"][number], "enforced_by">[]; memory: Rules["memory"]; checks: Omit<Rules["checks"][number], "receipt">[] };
  files: string[];
  diff: { old: string; new: string };
  branches: Omit<BranchRow, "run" | "task">[];
  worktrees: WorktreeRow[];
  counts: { decisions: number; memory: number; checks: number };
  /** Runs, asks and evidence that already happened when the preview opens. */
  history?: (h: History) => void;
}

interface History {
  run: (partial: Partial<Run> & { id: string; task: string }) => Run;
  ev: (run: string, kind: string, body: unknown, at?: number) => void;
  ask: (a: Ask) => void;
  /** `carried`: `tree` is an earlier one, and nothing in the check's scope changed since. */
  evidence: (e: Omit<Evidence, "id">, carried?: boolean) => void;
  judgment: (j: Omit<Judgment, "id">) => void;
  /** A scripted agent that keeps streaming while you watch. */
  live: (run: Run, script: { kind: string; body: unknown }[], every: number) => void;
}

const agents = [
  { name: "claude", command: "npx -y @agentclientprotocol/claude-agent-acp", source: "preset" },
  { name: "codex", command: "npx -y @agentclientprotocol/codex-acp", source: "preset" },
  { name: "gemini", command: "gemini --acp", source: "preset" },
  { name: "opencode", command: "opencode acp", source: "preset" },
  { name: "test", command: "kitsu-test-agent", source: "bundled" },
];

const order = ["needs_you", "working", "ready", "waiting", "quiet"];

// A stand-in for CheckDef::fingerprint: stable hex from the command.
function fingerprint(s: string): string {
  let h = 0x811c9dc5;
  let out = "";
  for (let round = 0; round < 2; round++)
    for (const c of s + round) {
      h ^= c.charCodeAt(0);
      h = Math.imul(h, 0x01000193) >>> 0;
      if (out.length < 16) out += (h & 0xff).toString(16).padStart(2, "0");
    }
  return out.padEnd(16, "0").slice(0, 16);
}

// kitsu::scope::glob_match: `**` any segments, `*` within one, `?` one character.
function globMatch(glob: string, path: string): boolean {
  const re = glob
    .split("/")
    .map((seg) => (seg === "**" ? "(?:[^/]+/)*" : seg.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, "[^/]*").replace(/\?/g, "[^/]") + "/"))
    .join("");
  return new RegExp(`^${re}$`).test(path + "/");
}
const refused = (detail: string) => ({ kind: "invalid", message: `not saved, it would break the task files: ${detail}` });

function makeRepo(id: string, seed: Seed) {
  const tasks = seed.tasks.map((t) => ({ ...t }));
  const questions = seed.questions.map((q) => ({ ...q }));
  const versions = new Map(tasks.map((t) => [t.id, "v1"]));
  const checkNames = seed.checks;
  const runs: Run[] = [];
  const events: RunEvent[] = [];
  const asks: Ask[] = [];
  const evidence: Evidence[] = [];
  const carried = new Map<number, string>();
  const judgments: Judgment[] = [];
  const info ={ id, root: seed.root, name: seed.name, branch: seed.branch, trusted: seed.trusted, initialized: seed.initialized ?? true };
  const changed = () => changedIn(id);
  let seen = 0;

  function run(partial: Partial<Run> & { id: string; task: string }): Run {
    const r: Run = {
      agent: "claude",
      base: "71dce3e6eed1e056367382bffa8a9aa4a5470d5c",
      branch: `kitsu/run/${partial.id}`,
      worktree: `${seed.root}/.git/kitsu/worktrees/${partial.id}`,
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

  seed.history?.({
    run,
    ev,
    ask: (a) => asks.push(a),
    evidence: (e, isCarried) => {
      const id = evidence.length + 1;
      evidence.push({ id, ...e });
      if (isCarried) carried.set(id, e.tree);
    },
    judgment: (j) => judgments.push({ id: judgments.length + 1, ...j }),
    live: (r, script, every) => {
      let step = 0;
      setInterval(() => {
        if (r.state !== "running" || step >= script.length) return;
        const s = script[step++]!;
        ev(r.id, s.kind, s.body);
        changed();
      }, every);
    },
  });

  const required = (t: MockTask) => [...new Set([...t.checks, ...Object.entries(seed.guards ?? {}).filter(([path]) => t.scope.includes(path)).map(([, c]) => c)])].sort();

  const latest = (name: string, runId?: string) => [...evidence].reverse().find((x) => x.check_name === name && (!runId || x.run === runId));

  const checkStatus = (name: string, runId?: string): CheckStatus => {
    const e = latest(name, runId);
    if (!e) return { status: "unverified" };
    const from = carried.get(e.id);
    return from ? { status: "carried", outcome: e.outcome, evidence: e.id, from_tree: from } : { status: "current", outcome: e.outcome, evidence: e.id };
  };

  // Same shape as kitsu::check::receipt: the evidence row behind a status.
  const receipt = (name: string, runId?: string): Receipt | null => {
    const e = latest(name, runId);
    if (!e) return null;
    const from = carried.get(e.id);
    return { check: name, evidence: e.id, command: e.command, fingerprint: fingerprint(e.command), tree: e.tree, outcome: e.outcome, exit_code: e.exit_code, duration_ms: e.duration_ms, started_at: e.started_at, run: e.run, binding: from ? "carried" : "current" };
  };
  const receiptsFor = (t: MockTask, runId: string) => required(t).flatMap((c) => receipt(c, runId) ?? []);

  // status::unchecked_paths: changed paths outside the scope of every check this change requires.
  const scopeOf = (name: string) => seed.rules.checks.find((c) => c.name === name)?.scope ?? [];
  const unguardedFor = (t: MockTask, changed: string[]) => changed.filter((p) => !required(t).some((c) => scopeOf(c).length === 0 || scopeOf(c).some((g) => globMatch(g, p))));

  // status::enforcing_checks, for the globs the preview uses: every glob of the
  // decision's scope is a guard glob, or sits under a `dir/**` guard.
  const enforcedBy = (scope: string[]) => {
    const guards = seed.rules.checks.filter((c) => c.guards.length);
    if (!scope.length) return [];
    const by = scope.map((g) => guards.filter((c) => c.guards.some((h) => h === g || (h.endsWith("/**") && g.startsWith(h.slice(0, -2))))).map((c) => c.name));
    return by.some((b) => !b.length) ? [] : [...new Set(by.flat())].sort();
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
      const failing = required(t).filter((c) => !["current", "carried"].includes(checkStatus(c, fin.id).status));
      const unguarded = unguardedFor(t, fin.changed ?? []);
      return {
        ...base,
        status: { kind: "review", run: fin.id, agent: fin.agent, verdict: failing.length ? "unverified" : "verified", failing: [], unguarded, receipts: receiptsFor(t, fin.id) },
        attention: "needs_you",
        reason: failing.length ? "ready for review, not verified yet" : "ready for review, checks pass",
      };
    }
    const failed = open[0];
    if (failed) return { ...base, status: { kind: "failed", run: failed.id, agent: failed.agent, detail: failed.detail ?? "failed" }, attention: "needs_you", reason: `${failed.agent} failed: ${failed.detail}` };
    const q = questions.filter((q) => q.open && q.blocks.includes(t.id)).map((q) => q.id);
    if (q.length) return { ...base, status: { kind: "blocked_by_question", questions: q }, attention: "needs_you", reason: `blocked on ${q.join(", ")}` };
    const deps = t.after.filter((d) => tasks.find((x) => x.id === d)?.state === "open");
    if (deps.length) return { ...base, status: { kind: "blocked_by_tasks", tasks: deps }, attention: "waiting", reason: `waiting on ${deps.join(", ")}` };
    return { ...base, status: { kind: "ready" }, attention: "ready", reason: "ready to start" };
  }

  const views = () => (info.initialized ? tasks.map(view) : []);

  const summary = (p: Project): ProjectSummary => {
    const v = views();
    const n = (a: string) => v.filter((x) => x.attention === a).length;
    return { ...p, branch: info.branch, trusted: info.trusted, initialized: info.initialized, needs_you: n("needs_you"), working: n("working"), ready: n("ready"), missing: false, error: null };
  };

  const digest = () => {
    const items: Overview["since"]["items"] = [];
    for (const a of asks.filter((a) => !a.answer)) {
      const r = runs.find((x) => x.id === a.run);
      items.push({ kind: "ask", run: a.run, task: r?.task ?? null, text: `the agent asks: ${a.request.title}`, params: { title: a.request.title } });
    }
    for (const r of runs.filter((r) => r.state === "finished" && !r.resolution)) items.push({ kind: "finished", run: r.id, task: r.task, text: "finished, ready for review", params: { stop_reason: r.stop_reason } });
    return items;
  };

  const tree = (): RepoTree => {
    const open = runs.filter((r) => !r.resolution);
    const branches: BranchRow[] = [
      ...seed.branches.map((b) => ({ ...b, current: b.name === info.branch, run: null, task: null })),
      ...open.map((r, i) => ({ name: r.branch, head: (r.snapshot ?? r.base).slice(0, 7), current: false, upstream: null, ahead: null, behind: null, gone: false, date: Math.round(r.created_at / 1000) - i, subject: `kitsu: snapshot of ${r.id}`, run: r.id, task: r.task })),
    ];
    const worktrees: WorktreeRow[] = [
      { path: seed.root, head: "71dce3e6eed1e056367382bffa8a9aa4a5470d5c", branch: info.branch, locked: false, prunable: false, owner: { kind: "main" } },
      ...open.map((r) => ({ path: r.worktree, head: r.snapshot ?? r.base, branch: r.branch, locked: false, prunable: false, owner: { kind: "run" as const, run: r.id, task: r.task, state: r.state } })),
      ...seed.worktrees,
    ];
    return { branch: info.branch, branches, worktrees };
  };

  const handlers: Record<string, Handler> = {
    open_repo: () => ({ ...info }),
    trust_repo: () => {
      info.trusted = true;
      changed();
      return { ...info };
    },
    init_repo: () => {
      info.initialized = true;
      changed();
      return { ...info };
    },
    overview: (): Overview => ({
      repo: { ...info },
      tasks: views().sort((a, b) => order.indexOf(a.attention) - order.indexOf(b.attention) || a.id.localeCompare(b.id)),
      asks: asks.filter((a) => !a.answer),
      since: { from_seq: seen, to_seq: seq, items: seen >= seq ? [] : digest() },
      agents,
      problems: [],
      counts: { ...seed.counts, open_questions: questions.filter((q) => q.open).length },
    }),
    mark_seen: (a) => {
      seen = a.seq as number;
    },
    task_detail: (a): TaskDetail => {
      const t = tasks.find((x) => x.id === a.id);
      if (!t) throw { kind: "not_found", message: `not found: task ${a.id}` };
      const req = required(t);
      const decisions = seed.rules.decisions.filter((d) => d.scope.some((s) => t.scope.includes(s)));
      return {
        required: req,
        task: { ...t, path: `.kitsu/tasks/${t.id}.md` },
        brief: {
          task: t.id,
          markdown: `# ${t.title}\n\n${t.body}\n\n## Done means\n${req.map((c) => `- Check \`${c}\` passes`).join("\n") || "- You accept it"}`,
          included: [
            { kind: "task", id: t.id, title: t.title, path: `.kitsu/tasks/${t.id}.md`, content_id: "a1", why: "the task" },
            ...decisions.map((d) => ({ kind: "decision", id: d.id, title: d.title, path: d.path, content_id: "c3", why: `task scope ${t.scope.join(", ")} overlaps ${d.scope.join(", ")}`, enforced_by: enforcedBy(d.scope) })),
          ],
          omitted: [],
          problems: [],
        },
        runs: runs.filter((r) => r.task === t.id).sort((x, y) => y.created_at - x.created_at),
        questions: questions.filter((q) => q.blocks.includes(t.id)),
        dirty_checkout: id === "payments",
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
      const changed = r.changed ?? t.scope.filter((s) => !s.includes("*"));
      const files = changed.map((path, i) => ({ path, added: 18 + i * 23, removed: i ? 0 : 6 }));
      const said = [...events].reverse().find((e) => e.run === r.id && e.kind === "agent.message");
      return {
        run: r.id,
        target: info.branch,
        target_head: "71dce3e",
        from: "71dce3e",
        files,
        protected: [],
        approval_token: null,
        checks: required(t).map((c) => [c, checkStatus(c, r.id)] as [string, CheckStatus]),
        receipts: receiptsFor(t, r.id),
        unguarded: unguardedFor(t, changed),
        claim: said ? String((said.body as { text?: string }).text ?? "") : null,
        judgments: judgments.filter((j) => j.run === r.id),
      };
    },
    file_diff: (a) => ({ path: a.path, old: seed.diff.old, new: seed.diff.new, protected: false }),
    start_run: (a) => {
      if (!info.trusted) throw { kind: "denied", message: "denied: trust this repository first; agents run its code" };
      const rid = "r" + Math.random().toString(36).slice(2, 8);
      const r = run({ id: rid, task: a.task as string, agent: a.agent as string, created_at: now(), note: (a.note as string) ?? null });
      const touched = tasks.find((x) => x.id === a.task)?.scope.find((s) => !s.includes("*")) ?? seed.files[0]!;
      ev(rid, "run.state", { to: "running" });
      let i = 0;
      const script = [
        { kind: "agent.message", body: { text: "Reading the relevant files." } },
        { kind: "agent.tool", body: { id: "t1", title: `Read ${touched}`, kind: "read", status: "completed", locations: [touched] } },
        { kind: "agent.tool", body: { id: "t2", title: `Edit ${touched}`, kind: "edit", status: "completed", locations: [touched] } },
        { kind: "agent.message", body: { text: "Done. The change is in place and the checks can run." } },
      ];
      const timer = setInterval(() => {
        if (r.state !== "running") return clearInterval(timer);
        const s = script[i++];
        if (!s) {
          r.state = "finished";
          r.stop_reason = "end_turn";
          r.snapshot = "abc1234";
          r.ended_at = now();
          r.changed = [touched];
          ev(rid, "run.state", { to: "finished", stop_reason: "end_turn" });
          clearInterval(timer);
        } else ev(rid, s.kind, s.body);
        changed();
      }, 1200);
      changed();
      return rid;
    },
    stop_run: (a) => {
      const r = runs.find((x) => x.id === a.id)!;
      r.state = "finished";
      r.stop_reason = "cancelled";
      r.cancel_requested = true;
      r.snapshot = "partial1";
      r.changed = r.changed ?? [seed.files[0]!];
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
      return { result: "applied", commit: "5231cde353aa", closed_task: !!a.closeTask, notes: [], unguarded: unguardedFor(t, r.changed ?? []) };
    },
    discard_run: (a) => {
      runs.find((x) => x.id === a.run)!.resolution = "discarded";
      changed();
      return [];
    },
    new_entity: (a) => {
      const eid = String(a.title).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 48);
      if (a.kind === "task") versions.set(eid, "v1");
      if (a.kind === "task") tasks.push({ id: eid, title: a.title as string, state: "open", scope: [...((a.scope as string[]) ?? [])], checks: [...((a.checks as string[]) ?? [])], after: [...((a.after as string[]) ?? [])], body: (a.body as string) ?? "" });
      changed();
      return { id: eid, path: `.kitsu/tasks/${eid}.md` };
    },
    plan: (): Plan => ({
      tasks: tasks.map((t) => ({ id: t.id, title: t.title, state: t.state, scope: t.scope, checks: t.checks, after: t.after, path: `.kitsu/tasks/${t.id}.md`, version: versions.get(t.id) ?? "v1" })),
      checks: checkNames,
    }),
    // Same refusals as kitsu::cli::update_task: unknown names, a stale file,
    // anything that would close a loop.
    update_task: (a) => {
      const t = tasks.find((x) => x.id === a.id);
      if (!t) throw { kind: "not_found", message: `not found: task ${a.id}` };
      if (a.version && a.version !== versions.get(t.id)) throw { kind: "conflict", message: `conflict: .kitsu/tasks/${t.id}.md changed on disk since it was read` };
      const clean = (v: unknown) => (v == null ? null : [...new Set((v as string[]).map((x) => x.trim()).filter(Boolean))]);
      const after = clean(a.after);
      const checks = clean(a.checks);
      const scope = clean(a.scope);
      const path = `.kitsu/tasks/${t.id}.md`;
      for (const d of after ?? []) if (!tasks.some((x) => x.id === d)) throw refused(`${path}: \`after\` names unknown task \`${d}\``);
      for (const c of checks ?? []) if (!checkNames.includes(c)) throw refused(`${path}: unknown check \`${c}\``);
      if (after) {
        // Would any new prerequisite, followed through its own, lead back here?
        const visited = new Set<string>();
        const reaches = (x: string): boolean => {
          if (x === t.id) return true;
          if (visited.has(x)) return false;
          visited.add(x);
          return (tasks.find((y) => y.id === x)?.after ?? []).some(reaches);
        };
        if (after.some(reaches)) throw refused(`${path}: dependency cycle through ${t.id}`);
        t.after = after;
      }
      if (checks) t.checks = checks;
      if (scope) t.scope = scope;
      const v = `v${Number((versions.get(t.id) ?? "v1").slice(1)) + 1}`;
      versions.set(t.id, v);
      changed();
      return v;
    },
    rules: (): Rules => ({
      decisions: seed.rules.decisions.map((d) => ({ ...d, enforced_by: enforcedBy(d.scope) })),
      memory: seed.rules.memory,
      // The checkout's latest evidence for each check, like status_at on the working tree.
      checks: seed.rules.checks.map((c) => {
        const r = receipt(c.name);
        const s = c.status;
        if (!r || s.status === "unverified" || s.status === "missing") return { ...c, receipt: null };
        return { ...c, receipt: { ...r, outcome: s.outcome, exit_code: s.outcome === "pass" ? 0 : 1, binding: s.status } };
      }),
      questions: questions.map((q) => ({ ...q, path: `.kitsu/questions/${q.id}.md` })),
    }),
    run_checks: () => {
      if (!info.trusted) throw { kind: "denied", message: "trust this repository first; checks run its code" };
      return [];
    },
    read_file: (a) => ({ path: a.path, text: seed.diff.new, version: "v1" }),
    write_file: () => "v2",
    list_files: () => seed.files,
    git_view: tree,
  };

  return { info, handlers, summary };
}

// payments: the retry-storm example, mid-afternoon. One finished run to
// review, one agent working, one asking. -------------------------------------

const payments = makeRepo("a4493a668f92", {
  root: "/home/you/payments",
  name: "payments",
  branch: "main",
  trusted: true,
  checks: ["idempotency", "refunds", "retries"],
  guards: { "payments.py": "idempotency" },
  counts: { decisions: 3, memory: 3, checks: 3 },
  tasks: [
    { id: "bounded-retries", title: "Stop retrying forever when the upstream times out", state: "open", scope: ["payments.py"], checks: ["retries"], after: ["persist-idempotency-keys"], body: "Last Tuesday the upstream browned out for 20 minutes and every web worker sat in `charge()` spinning on timeouts. That made our outage longer than theirs." },
    { id: "refund-endpoint", title: "Add refunds that can't be issued twice", state: "open", scope: ["refunds.py", "payments.py"], checks: ["refunds"], after: [], body: "Support asked for partial refunds. Same rules as charges: a lost response must not turn into two refunds." },
    { id: "metrics-export", title: "Export retry and timeout counts to the metrics endpoint", state: "open", scope: ["metrics.py"], checks: [], after: [], body: "We had no idea how often we were retrying until the outage." },
    { id: "webhook-signatures", title: "Verify webhook signatures from the upstream", state: "open", scope: ["webhooks.py"], checks: ["webhooks"], after: [], body: "Right now anybody can POST a fake 'charge.succeeded'." },
    { id: "dashboard-latency", title: "Show upstream latency on the ops dashboard", state: "open", scope: ["ops/**"], checks: [], after: ["metrics-export"], body: "Needs the metrics export first." },
    { id: "remove-legacy-client", title: "Delete the legacy SOAP client", state: "done", scope: ["legacy/**"], checks: [], after: [], body: "" },
    { id: "persist-idempotency-keys", title: "Keep idempotency keys across worker restarts", state: "done", scope: ["payments.py", "store.py"], checks: ["idempotency"], after: [], body: "Keys lived in memory, so a deploy mid-retry lost them." },
    { id: "partial-refunds", title: "Allow partial refunds up to the charged amount", state: "open", scope: ["refunds.py"], checks: ["refunds"], after: ["refund-endpoint"], body: "Several partial refunds may never add up to more than the charge." },
    { id: "refund-emails", title: "Email the customer when a refund goes through", state: "open", scope: ["notify/**"], checks: [], after: ["partial-refunds", "webhook-signatures"], body: "Only on the upstream's signed 'refund.succeeded', never on our own request." },
    { id: "retry-alerts", title: "Page on-call when retries spike", state: "open", scope: ["ops/alerts.yml"], checks: [], after: ["metrics-export", "bounded-retries"], body: "Alert on the retry rate, not on single timeouts." },
    { id: "minor-units", title: "Store amounts in minor units, not floats", state: "open", scope: ["payments.py", "refunds.py"], checks: ["retries", "refunds"], after: [], body: "0.1 + 0.2 already cost us a cent in reconciliation." },
    { id: "ledger-reconcile", title: "Reconcile the ledger against upstream payouts nightly", state: "open", scope: ["ledger/**"], checks: [], after: ["minor-units", "partial-refunds"], body: "Report any charge or refund that doesn't match a payout line." },
  ],
  questions: [{ id: "webhook-secret-rotation", title: "Do we have to accept both old and new webhook secrets during rotation?", open: true, blocks: ["webhook-signatures"], answer: null, body: "If yes, verification needs a list of secrets and a way to retire one." }],
  rules: {
    decisions: [
      { id: "one-key-per-charge", title: "One idempotency key per logical charge, reused by every retry", state: "accepted", scope: ["payments.py"], rejected: [], body: "The upstream can charge the card and then lose the response. The only thing that makes a retry safe is sending the same `idempotency_key` again.", path: ".kitsu/decisions/one-key-per-charge.md" },
      { id: "retry-budget", title: "Retry charges at most 3 times, only with an idempotency key", state: "accepted", scope: ["payments.py"], rejected: ["Infinite retry: turns an upstream brownout into our outage", "Circuit breaker for now: one caller, no evidence of long outages", "Retrying without a key: a lost response becomes a second charge"], body: "Three attempts total, with jittered backoff between them.", path: ".kitsu/decisions/retry-budget.md" },
      // No check guards the runbook: this one is a note, and the UI says so.
      { id: "retries-are-logged", title: "Every retry is logged with its charge id, and the runbook says how to read it", state: "accepted", scope: ["payments.py", "docs/runbook.md"], rejected: [], body: "On-call greps the logs for the charge id first.", path: ".kitsu/decisions/retries-are-logged.md" },
    ],
    memory: [
      { id: "upstream-dedupes", title: "The upstream dedupes idempotency keys for 24 hours", kind: "fact", scope: ["payments.py"], anchors: ["payments.py"], by: "you", run: null, body: "From their API docs, section Idempotency. A retry after 24h is a new charge.", path: ".kitsu/memory/upstream-dedupes.md", freshness: { status: "current" }, personal: false, state: "current", superseded_by: null, reason: null, key: null },
      { id: "fake-upstream-sleeps", title: "fake_upstream sleeps for real unless you pass sleep=", kind: "gotcha", scope: ["test_*.py"], anchors: ["fake_upstream.py"], by: "claude", run: "r7k2mq", body: "Tests take 20s instead of 0.2s if you forget it.", path: ".kitsu/memory/fake-upstream-sleeps.md", freshness: { status: "stale", changed: ["fake_upstream.py"], since: "71dce3e" }, personal: false, state: "current", superseded_by: null, reason: null, key: null },
      { id: "personal/small-commits", title: "Small commits, one idea each", kind: "preference", scope: [], anchors: [], by: null, run: null, body: "", path: "~/.config/kitsu/memory/small-commits.md", freshness: { status: "unanchored" }, personal: true, state: "current", superseded_by: null, reason: null, key: null },
    ],
    checks: [
      { name: "retries", run: "python3 -m unittest -q test_retries", scope: ["payments.py", "test_retries.py", "fake_upstream.py"], guards: [], why: null, status: { status: "stale", outcome: "fail", evidence: 1, changed: ["payments.py"], more: 0 } },
      { name: "idempotency", run: "python3 -m unittest -q test_idempotency", scope: ["payments.py"], guards: ["payments.py"], why: "A retry after a lost response must not charge the card twice", status: { status: "stale", outcome: "fail", evidence: 2, changed: ["payments.py"], more: 0 } },
      { name: "refunds", run: "python3 -m unittest -q test_refunds", scope: [], guards: [], why: null, status: { status: "unverified" } },
    ],
  },
  files: ["payments.py", "docs/runbook.md", "fake_upstream.py", "test_idempotency.py", "test_retries.py", "metrics.py", "refunds.py", ".kitsu/kitsu.toml", ".kitsu/tasks/bounded-retries.md", ".kitsu/decisions/one-key-per-charge.md", ".kitsu/decisions/retry-budget.md"],
  diff: {
    old: '"""Charges a card through an upstream payment API."""\n\nimport uuid\n\n\nclass Timeout(Exception):\n    pass\n\n\ndef charge(upstream, card, amount):\n    # Upstream has been flaky, so keep trying until it goes through.\n    while True:\n        try:\n            return upstream.post("/charges", card=card, amount=amount)\n        except Timeout:\n            continue\n',
    new: '"""Charges a card through an upstream payment API."""\n\nimport random\nimport time\nimport uuid\n\n\nclass Timeout(Exception):\n    pass\n\n\nMAX_ATTEMPTS = 3\n\n\ndef charge(upstream, card, amount, sleep=time.sleep):\n    # One key for the whole logical charge. If the upstream charged us and\n    # the response got lost, the retry carries the same key and is a no-op.\n    key = str(uuid.uuid4())\n    for attempt in range(MAX_ATTEMPTS):\n        try:\n            return upstream.post("/charges", card=card, amount=amount, idempotency_key=key)\n        except Timeout:\n            if attempt == MAX_ATTEMPTS - 1:\n                raise\n            sleep(random.uniform(0, 0.05 * 2**attempt))\n',
  },
  branches: [
    { name: "fix/refund-rounding", head: "9b1c2d4", current: false, upstream: "origin/fix/refund-rounding", ahead: 2, behind: 0, gone: false, date: days(1), subject: "Round refunds half-even, like charges" },
    { name: "main", head: "71dce3e", current: true, upstream: "origin/main", ahead: 0, behind: 0, gone: false, date: days(0.1), subject: "Keep idempotency keys across worker restarts" },
    { name: "spike/circuit-breaker", head: "4e0aa17", current: false, upstream: null, ahead: null, behind: null, gone: false, date: days(9), subject: "WIP: breaker around charge()" },
  ],
  worktrees: [{ path: "/home/you/payments-spike", head: "4e0aa17d3c", branch: "spike/circuit-breaker", locked: false, prunable: false, owner: { kind: "yours" } }],
  history: (h) => {
    // A finished, verified attempt waiting for review.
    const good = h.run({ id: "r7k2mq", task: "bounded-retries", state: "finished", stop_reason: "end_turn", snapshot: "c0ffee1", ended_at: minutes(3), created_at: minutes(9), changed: ["payments.py", "docs/runbook.md"], usage: { input: 18400, output: 2900, cached_read: 61000, total: 82300, context_used: 41000, context_size: 200000, cost: 0.19, currency: "USD" } });
    h.ev(good.id, "run.state", { to: "running" }, minutes(9));
    h.ev(good.id, "agent.plan", { entries: [{ content: "Bound the retry loop to 3 attempts", status: "completed" }, { content: "Create the idempotency key once, before the first attempt", status: "completed" }, { content: "Run the checks", status: "completed" }] }, minutes(8));
    h.ev(good.id, "agent.message", { text: "Reading the charge path and the retry loop first." }, minutes(8));
    h.ev(good.id, "agent.tool", { id: "t1", title: "Read payments.py", kind: "read", status: "completed", locations: ["payments.py"] }, minutes(8));
    h.ev(good.id, "agent.tool", { id: "t2", title: "Edit payments.py", kind: "edit", status: "completed", locations: ["payments.py"] }, minutes(6));
    h.ev(good.id, "agent.tool", { id: "t3", title: "Edit docs/runbook.md", kind: "edit", status: "completed", locations: ["docs/runbook.md"] }, minutes(5.5));
    // Triage let a test run through on the judge's word: a judgment, stored as one.
    h.judgment({ run: good.id, purpose: "permission", kind: "yes_no", outcome: "answered", answers: { low_risk: { type: "yes_no", p_yes: 0.96 } }, reason: null, model: "jev-latest", latency_ms: 412, created_at: minutes(5) });
    h.ev(good.id, "permission", { title: "Run `python3 -m unittest -q test_retries`", kind: "execute", decision: "allow_once", by: "judge", judgment: 1, p_yes: 0.96, threshold: 0.9 }, minutes(5));
    h.ev(good.id, "agent.message", { text: "Bounded to 3 attempts with jittered backoff; the key is created once per charge so a lost response can't double-charge. Updated the runbook too." }, minutes(4));
    h.ev(good.id, "run.state", { to: "finished", stop_reason: "end_turn" }, minutes(3));
    h.evidence({ check_name: "retries", command: "python3 -m unittest -q test_retries", tree: "3f9a2c1d7be04e58a1c6", tree_after: null, outcome: "pass", exit_code: 0, duration_ms: 1840, run: good.id, started_at: minutes(3) });
    h.ev(good.id, "check.done", { check: "retries", outcome: "pass" }, minutes(3));
    // Ran on an earlier snapshot; nothing it looks at changed since.
    h.evidence({ check_name: "idempotency", command: "python3 -m unittest -q test_idempotency", tree: "9e41b7c20d5a33f1c0de", tree_after: null, outcome: "pass", exit_code: 0, duration_ms: 610, run: good.id, started_at: minutes(5) }, true);
    h.ev(good.id, "check.done", { check: "idempotency", outcome: "pass" }, minutes(5));

    // An agent currently working, streaming as you watch.
    const live = h.run({ id: "r9fh3p", task: "refund-endpoint", agent: "codex", created_at: minutes(2) });
    h.ev(live.id, "run.state", { to: "running" }, minutes(2));
    h.ev(live.id, "agent.plan", { entries: [{ content: "Add refund() next to charge()", status: "in_progress" }, { content: "Reuse the idempotency key rule", status: "pending" }, { content: "Tests for partial refunds", status: "pending" }] }, minutes(2));
    h.ev(live.id, "agent.tool", { id: "t1", title: "Read payments.py", kind: "read", status: "completed", locations: ["payments.py"] }, minutes(1.5));
    h.ev(live.id, "agent.message", { text: "The charge path already creates one key per logical operation. Refunds need the same, keyed by (charge id, refund request id)." }, minutes(1));
    h.live(
      live,
      [
        { kind: "agent.tool", body: { id: "t2", title: "Edit payments.py", kind: "edit", status: "in_progress", locations: ["payments.py"] } },
        { kind: "agent.tool", body: { id: "t2", title: "Edit payments.py", kind: "edit", status: "completed", locations: ["payments.py"] } },
        { kind: "agent.message", body: { text: "Added refund() with the key created before the first attempt. Writing tests for the lost-response case." } },
        { kind: "agent.tool", body: { id: "t3", title: "Write test_refunds.py", kind: "edit", status: "completed", locations: ["test_refunds.py"] } },
      ],
      2500,
    );

    // One waiting on you.
    const asking = h.run({ id: "r3xw8d", task: "metrics-export", agent: "claude", created_at: minutes(1) });
    h.ev(asking.id, "run.state", { to: "running" }, minutes(1));
    h.ev(asking.id, "agent.tool", { id: "t1", title: "Edit metrics.py", kind: "edit", status: "completed", locations: ["metrics.py"] }, minutes(0.6));
    const request = { title: "Run `pip install prometheus-client`", kind: "execute", options: [{ optionId: "allow", name: "Allow once", kind: "allow_once" }, { optionId: "deny", name: "Deny", kind: "reject_once" }] };
    h.ask({ id: 41, run: asking.id, request, answer: null, created_at: minutes(0.4) });
    h.ev(asking.id, "ask.open", { ask: 41, request }, minutes(0.4));
  },
});

// web: the storefront, on a feature branch. One review, one failure, one
// agent working. --------------------------------------------------------------

const web = makeRepo("267072f47daf", {
  root: "/home/you/code/web",
  name: "web",
  branch: "feat/checkout-redesign",
  trusted: true,
  checks: ["e2e", "typecheck", "unit"],
  guards: { "src/checkout/**": "e2e" },
  counts: { decisions: 3, memory: 1, checks: 3 },
  tasks: [
    { id: "checkout-a11y", title: "Make the checkout form usable with a screen reader", state: "open", scope: ["src/checkout/**"], checks: ["e2e"], after: [], body: "Labels are placeholders, errors aren't announced, and the pay button is a div." },
    { id: "design-tokens", title: "Move colors to design tokens so dark mode is one switch", state: "open", scope: ["src/styles/tokens.css", "src/styles/theme.ts"], checks: ["unit"], after: [], body: "Forty hex codes spread across components. Tokens first, then dark mode is a second file." },
    { id: "flaky-cart-e2e", title: "Fix the flaky cart end-to-end test", state: "open", scope: ["e2e/cart.spec.ts"], checks: ["e2e"], after: [], body: "Fails one run in ten on CI: it clicks before the cart has hydrated." },
    { id: "lazy-images", title: "Lazy-load product images below the fold", state: "open", scope: ["src/product/**"], checks: ["unit"], after: [], body: "The listing page downloads 4 MB of images nobody scrolls to." },
    { id: "bundle-budget", title: "Fail the build when the bundle grows past 250 KB", state: "open", scope: ["vite.config.ts"], checks: [], after: [], body: "It crept from 180 KB to 310 KB in two months without anyone deciding that." },
    { id: "local-prices", title: "Format prices in the shopper's locale", state: "open", scope: ["src/checkout/price.ts"], checks: ["unit"], after: ["checkout-a11y"], body: "Intl.NumberFormat with the shop's currency, not string concatenation." },
    { id: "old-router", title: "Drop the hand-rolled router", state: "done", scope: ["src/router/**"], checks: [], after: [], body: "" },
  ],
  questions: [],
  rules: {
    decisions: [
      { id: "no-css-in-js", title: "Plain CSS with custom properties, no CSS-in-JS", state: "accepted", scope: ["src/styles/**"], rejected: ["styled-components: runtime cost on every render", "Tailwind: the design system already has names for things"], body: "Tokens are custom properties; components read them.", path: ".kitsu/decisions/no-css-in-js.md" },
    ],
    memory: [],
    checks: [
      { name: "unit", run: "npm test", scope: ["src/**"], guards: [], why: null, status: { status: "current", outcome: "pass", evidence: 1 } },
      { name: "typecheck", run: "npm run check", scope: ["src/**"], guards: [], why: null, status: { status: "current", outcome: "pass", evidence: 2 } },
      { name: "e2e", run: "npx playwright test", scope: ["src/checkout/**", "e2e/**"], guards: ["src/checkout/**"], why: "Checkout is where a broken form costs money", status: { status: "unverified" } },
    ],
  },
  files: ["src/checkout/Form.svelte", "src/checkout/price.ts", "src/styles/tokens.css", "src/product/Listing.svelte", "e2e/cart.spec.ts", "vite.config.ts", ".kitsu/kitsu.toml"],
  diff: {
    old: '<div class="pay" on:click={pay}>Pay</div>\n<input placeholder="Card number" />\n',
    new: '<label for="card">Card number</label>\n<input id="card" autocomplete="cc-number" aria-describedby="card-error" />\n<p id="card-error" role="alert">{error}</p>\n<button class="pay" type="submit">Pay</button>\n',
  },
  branches: [
    { name: "feat/checkout-redesign", head: "c3d9e01", current: true, upstream: "origin/feat/checkout-redesign", ahead: 3, behind: 0, gone: false, date: days(0.05), subject: "Checkout: one column on small screens" },
    { name: "main", head: "8a2f6b0", current: false, upstream: "origin/main", ahead: 0, behind: 4, gone: false, date: days(2), subject: "Release 2026.09.2" },
    { name: "chore/deps", head: "11aa2e9", current: false, upstream: "origin/chore/deps", ahead: 0, behind: 0, gone: true, date: days(21), subject: "Bump vite" },
  ],
  worktrees: [{ path: "/home/you/code/web-main", head: "8a2f6b0c1d", branch: "main", locked: false, prunable: false, owner: { kind: "yours" } }],
  history: (h) => {
    const working = h.run({ id: "rw4k9n", task: "checkout-a11y", agent: "claude", created_at: minutes(6) });
    h.ev(working.id, "run.state", { to: "running" }, minutes(6));
    h.ev(working.id, "agent.plan", { entries: [{ content: "Real labels and a <button>", status: "completed" }, { content: "Announce errors with role=alert", status: "in_progress" }, { content: "Run the e2e check", status: "pending" }] }, minutes(5));
    h.ev(working.id, "agent.tool", { id: "t1", title: "Edit src/checkout/Form.svelte", kind: "edit", status: "completed", locations: ["src/checkout/Form.svelte"] }, minutes(3));
    h.live(working, [{ kind: "agent.message", body: { text: "Errors are announced now. Running the e2e check on the change." } }], 4000);

    const review = h.run({ id: "rw2t7c", task: "design-tokens", agent: "codex", state: "finished", stop_reason: "end_turn", snapshot: "d00dfee", created_at: minutes(40), ended_at: minutes(22), changed: ["src/styles/tokens.css", "src/styles/theme.ts"] });
    h.ev(review.id, "run.state", { to: "running" }, minutes(40));
    h.ev(review.id, "agent.message", { text: "Replaced 41 hex codes with 12 tokens; the dark theme is a second block of the same names." }, minutes(23));
    h.ev(review.id, "run.state", { to: "finished", stop_reason: "end_turn" }, minutes(22));
    h.evidence({ check_name: "unit", command: "npm test", tree: "b27d5e90a4c1f38e6d02", tree_after: null, outcome: "pass", exit_code: 0, duration_ms: 2300, run: review.id, started_at: minutes(22) });

    const flaky = h.run({ id: "rw8p1x", task: "flaky-cart-e2e", agent: "gemini", state: "failed", detail: "the agent process exited (code 1) before finishing", created_at: minutes(55), ended_at: minutes(50) });
    h.ev(flaky.id, "run.state", { to: "failed", detail: flaky.detail }, minutes(50));
  },
});

// infra: not trusted yet, so it opens read-only; one question blocks a task.

const infra = makeRepo("7a95776abd12", {
  root: "/home/you/code/infra",
  name: "infra",
  branch: "main",
  trusted: false,
  checks: ["plan", "tflint"],
  counts: { decisions: 1, memory: 0, checks: 2 },
  tasks: [
    { id: "state-lock", title: "Lock Terraform state so two applies can't race", state: "open", scope: ["terraform/backend.tf"], checks: ["plan"], after: [], body: "Two applies at once corrupted state in March. DynamoDB locking, same as the other stacks." },
    { id: "rotate-db-creds", title: "Rotate the database credentials every month", state: "open", scope: ["terraform/db/**"], checks: ["plan"], after: [], body: "They haven't changed since 2023." },
    { id: "cost-alerts", title: "Alert when the monthly bill jumps by a fifth", state: "open", scope: ["terraform/billing.tf"], checks: ["tflint"], after: [], body: "We found the NAT gateway bill three weeks late." },
    { id: "k8s-upgrade", title: "Upgrade the cluster to 1.34", state: "open", scope: ["terraform/eks/**"], checks: ["plan"], after: ["state-lock"], body: "Not while two people can apply at once." },
  ],
  questions: [{ id: "creds-downtime", title: "Can the app tolerate a few seconds of failed connections during rotation?", open: true, blocks: ["rotate-db-creds"], answer: null, body: "If not, we need two users and a switch-over instead of an in-place change." }],
  rules: {
    decisions: [{ id: "plan-before-apply", title: "Every change is a reviewed plan before it's applied", state: "accepted", scope: ["terraform/**"], rejected: ["Apply from laptops: nobody sees what changed"], body: "CI posts the plan; a human applies it.", path: ".kitsu/decisions/plan-before-apply.md" }],
    memory: [],
    checks: [
      { name: "plan", run: "terraform plan -detailed-exitcode", scope: ["terraform/**"], guards: [], why: null, status: { status: "unverified" } },
      { name: "tflint", run: "tflint --recursive", scope: ["terraform/**"], guards: [], why: null, status: { status: "unverified" } },
    ],
  },
  files: ["terraform/backend.tf", "terraform/billing.tf", "terraform/db/main.tf", "terraform/eks/cluster.tf", ".kitsu/kitsu.toml"],
  diff: { old: 'terraform {\n  backend "s3" {\n    bucket = "acme-tf"\n  }\n}\n', new: 'terraform {\n  backend "s3" {\n    bucket         = "acme-tf"\n    dynamodb_table = "acme-tf-lock"\n  }\n}\n' },
  branches: [
    { name: "main", head: "5f5e0c2", current: true, upstream: "origin/main", ahead: 0, behind: 0, gone: false, date: days(3), subject: "Tag every resource with its cost center" },
    { name: "tf/provider-6", head: "e81b774", current: false, upstream: "origin/tf/provider-6", ahead: 0, behind: 0, gone: true, date: days(30), subject: "Try the AWS provider 6.0" },
  ],
  worktrees: [],
});

// The workspaces list, as `workspaces.toml` would hold it. ---------------------

type MockRepo = ReturnType<typeof makeRepo>;
const repos = new Map<string, MockRepo>([payments, web, infra].map((r) => [r.info.id, r]));
const listed: Project[] = [payments, web, infra].map((r) => ({ id: r.info.id, name: r.info.name, root: r.info.root }));

function addFolder(path: string, name: string | null): Project {
  const p = path.trim().replace(/^~(?=\/)/, "/home/you").replace(/\/+$/, "");
  if (!p.startsWith("/")) throw { kind: "invalid", message: `${path} is not a full path; start it with / or ~/` };
  const hit = listed.find((x) => x.root === p);
  if (hit) throw { kind: "conflict", message: `conflict: ${p} is already in the list as ${hit.name}` };
  const inside = listed.find((x) => p.startsWith(x.root + "/"));
  if (inside) throw { kind: "invalid", message: `${p} is inside the repository ${inside.root}, not a repository of its own. Add ${inside.root} instead` };
  if (!p.startsWith("/home/you/")) throw { kind: "invalid", message: `${p} isn't a git repository. Kitsu keeps its state next to git's: run \`git init\` there first` };
  const folder = p.split("/").pop()!;
  const id = Math.random().toString(16).slice(2, 14).padEnd(12, "0");
  const repo = makeRepo(id, {
    root: p,
    name: folder,
    branch: "main",
    trusted: false,
    initialized: false,
    checks: [],
    counts: { decisions: 0, memory: 0, checks: 0 },
    tasks: [],
    questions: [],
    rules: { decisions: [], memory: [], checks: [] },
    files: ["README.md"],
    diff: { old: "", new: "" },
    branches: [{ name: "main", head: "0badc0d", current: true, upstream: null, ahead: null, behind: null, gone: false, date: days(0.01), subject: "Initial commit" }],
    worktrees: [],
  });
  repos.set(id, repo);
  const entry = { id, name: name?.trim() || folder, root: p };
  listed.push(entry);
  return entry;
}

const lists: Record<string, Handler> = {
  launch_repo: () => listed.find((p) => p.id === payments.info.id) ?? null,
  list_workspaces: () => listed,
  workspace_overview: () => listed.map((p) => repos.get(p.id)!.summary(p)),
  add_workspace: (a) => addFolder(String(a.path ?? ""), (a.name as string | null) ?? null),
  remove_workspace: (a) => {
    const i = listed.findIndex((p) => p.id === a.repo);
    if (i < 0) throw { kind: "not_found", message: `not found: project ${a.repo} is not in the list` };
    return listed.splice(i, 1)[0];
  },
  rename_workspace: (a) => {
    const p = listed.find((x) => x.id === a.repo);
    if (!p) throw { kind: "not_found", message: `not found: project ${a.repo} is not in the list` };
    const name = String(a.name ?? "").trim();
    if (name.length > 64) throw { kind: "invalid", message: "name is longer than 64 characters" };
    p.name = name || p.root.split("/").pop()!;
    return p;
  },
  move_workspace: (a) => {
    const i = listed.findIndex((p) => p.id === a.repo);
    if (i < 0) throw { kind: "not_found", message: `not found: project ${a.repo} is not in the list` };
    const [p] = listed.splice(i, 1);
    listed.splice(Math.min(Number(a.index), listed.length), 0, p!);
  },
};

export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  await new Promise((r) => setTimeout(r, 15));
  const list = lists[cmd];
  if (list) return structuredClone(list(args)) as T;
  const repo = typeof args.repo === "string" && listed.some((p) => p.id === args.repo) ? repos.get(args.repo) : undefined;
  if (!repo) throw { kind: "no_repo", message: `project ${String(args.repo)} is not in the list` };
  const h = repo.handlers[cmd];
  if (!h) throw { kind: "invalid", message: `mock: no handler for ${cmd}` };
  return structuredClone(h(args)) as T;
}
