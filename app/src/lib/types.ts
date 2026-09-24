// Shapes of what the Rust side sends. Kept by hand next to the commands in
// src-tauri/src/commands.rs; the mock in mock.ts uses the same types, so a
// drift shows up as a type error there first.

export type Attention = "needs_you" | "working" | "ready" | "waiting" | "quiet";

export type Verdict = "verified" | "failing" | "unverified" | "empty" | "unknown";

export type Status =
  | { kind: "running"; run: string; agent: string; stopping: boolean }
  | { kind: "asking"; run: string; agent: string; asks: number }
  | { kind: "review"; run: string; agent: string; verdict: Verdict; failing: string[] }
  | { kind: "failed"; run: string; agent: string; detail: string }
  | { kind: "interrupted"; run: string; agent: string }
  | { kind: "blocked_by_question"; questions: string[] }
  | { kind: "blocked_by_tasks"; tasks: string[] }
  | { kind: "ready" }
  | { kind: "done" }
  | { kind: "dropped" };

export interface TaskView {
  id: string;
  title: string;
  status: Status;
  attention: Attention;
  reason: string;
  path: string;
  others: number;
}

export interface Repo {
  root: string;
  name: string;
  branch: string | null;
  trusted: boolean;
  initialized: boolean;
}

export interface AskOption {
  optionId: string;
  name?: string;
  kind?: string;
}

export interface Ask {
  id: number;
  run: string;
  request: { title: string; kind?: string; locations?: { path: string }[]; options: AskOption[] };
  answer: string | null;
  created_at: number;
}

export interface DigestItem {
  kind: string;
  run: string | null;
  task: string | null;
  text: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  params: any;
}

export interface Digest {
  from_seq: number;
  to_seq: number;
  items: DigestItem[];
}

export interface Agent {
  name: string;
  command: string;
  source: string;
}

export interface Overview {
  repo: Repo;
  tasks: TaskView[];
  asks: Ask[];
  since: Digest;
  agents: Agent[];
  problems: { path: string; detail: string }[];
  counts: { invariants: number; decisions: number; open_questions: number; checks: number; memory?: number };
}

export type RunState = "starting" | "running" | "stopping" | "finished" | "failed" | "interrupted";

export interface Run {
  id: string;
  task: string;
  agent: string;
  base: string;
  branch: string;
  worktree: string;
  state: RunState;
  stop_reason: string | null;
  detail: string | null;
  cancel_requested: boolean;
  snapshot: string | null;
  resolution: "accepted" | "discarded" | null;
  from_run: string | null;
  note: string | null;
  created_at: number;
  ended_at: number | null;
  changed: string[] | null;
  /** What the agent reported; absent fields were not reported. */
  usage: Usage | null;
}

export interface Usage {
  input?: number;
  output?: number;
  cached_read?: number;
  cached_write?: number;
  thought?: number;
  total?: number;
  context_used?: number;
  context_size?: number;
  cost?: number;
  currency?: string;
}

export interface RunEvent {
  seq: number;
  run: string | null;
  at: number;
  kind: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  body: any;
}

export type Outcome = "pass" | "fail" | "timeout" | "error";

export interface Evidence {
  id: number;
  check_name: string;
  command: string;
  tree: string;
  tree_after: string | null;
  outcome: Outcome;
  exit_code: number | null;
  duration_ms: number;
  run: string | null;
  started_at: number;
}

export type CheckStatus =
  | { status: "current"; outcome: Outcome; evidence: number }
  | { status: "carried"; outcome: Outcome; evidence: number; from_tree: string }
  | { status: "stale"; outcome: Outcome; evidence: number; changed: string[]; more: number }
  | { status: "unverified" }
  | { status: "missing" };

export interface BriefItem {
  kind: string;
  id: string;
  title: string;
  path: string;
  content_id: string;
  why: string;
}

export interface Brief {
  task: string;
  markdown: string;
  included: BriefItem[];
  omitted: { kind: string; id: string; reason: string; reacquire: string }[];
  problems: string[];
}

export interface TaskDetail {
  task: { id: string; title: string; state: "open" | "done" | "dropped"; scope: string[]; checks: string[]; after: string[]; body: string; path: string };
  brief: Brief;
  runs: Run[];
  questions: { id: string; title: string; open: boolean; answer: string | null; body: string }[];
  dirty_checkout: boolean;
}

export interface RunDetail {
  run: Run;
  events: RunEvent[];
  evidence: Evidence[];
}

export interface FileStat {
  path: string;
  added: number | null;
  removed: number | null;
}

export interface Review {
  run: string;
  target: string;
  target_head: string;
  from: string;
  files: FileStat[];
  protected: string[];
  approval_token: string | null;
  checks: [string, CheckStatus][];
}

export type Accepted =
  | { result: "applied"; commit: string; closed_task: boolean; notes: string[] }
  | { result: "needs_approval"; paths: string[]; token: string }
  | { result: "conflict"; paths: string[] }
  | { result: "checks_failed"; failing: string[]; candidate: string };

export interface Rules {
  invariants: { id: string; title: string; active: boolean; scope: string[]; checks: { name: string; status: CheckStatus }[]; decision: string | null; body: string; path: string }[];
  decisions: { id: string; title: string; state: string; scope: string[]; rejected: string[]; body: string; path: string }[];
  questions: { id: string; title: string; open: boolean; blocks: string[]; answer: string | null; body: string; path: string }[];
  memory: MemoryNote[];
  checks: { name: string; run: string; scope: string[]; status: CheckStatus }[];
}

export type MemoryKind = "fact" | "gotcha" | "convention" | "preference" | "lesson";

export type Freshness =
  | { status: "current" }
  | { status: "stale"; changed: string[]; since: string }
  | { status: "unanchored" }
  | { status: "uncommitted" };

export interface MemoryNote {
  id: string;
  title: string;
  kind: MemoryKind;
  scope: string[];
  anchors: string[];
  by: string | null;
  run: string | null;
  body: string;
  path: string;
  freshness: Freshness | null;
  personal: boolean;
  state: "current" | "retired";
  superseded_by: string | null;
  reason: string | null;
  key: string | null;
}

export interface FileText {
  path: string;
  text: string;
  version: string;
}

export interface FileDiff {
  path: string;
  old: string | null;
  new: string | null;
  protected: boolean;
}

export interface UiError {
  kind: string;
  message: string;
}

/** An entry in the command palette. */
export interface Command {
  id: string;
  title: string;
  hint?: string;
  group: "commands" | "tasks" | "files";
  run: () => void;
}
