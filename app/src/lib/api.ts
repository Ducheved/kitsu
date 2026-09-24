// The only way the UI talks to the system. In the desktop app this is
// Tauri IPC to the typed commands in src-tauri/src/commands.rs. Opened in a
// plain browser (vite dev, UI tests) it runs against fixture data instead,
// and the window says so.

import type {
  Accepted,
  FileDiff,
  FileText,
  Overview,
  Repo,
  Review,
  Rules,
  RunDetail,
  TaskDetail,
  Evidence,
} from "./types";

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
type Listen = (event: string, cb: () => void) => Promise<() => void>;

export const native = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let invokeImpl: Invoke;
let listenImpl: Listen;

if (native) {
  const core = await import("@tauri-apps/api/core");
  const ev = await import("@tauri-apps/api/event");
  invokeImpl = core.invoke;
  listenImpl = async (event, cb) => ev.listen(event, () => cb());
} else {
  const mock = await import("./mock");
  invokeImpl = mock.invoke;
  listenImpl = mock.listen;
}

const call = <T>(cmd: string, args?: Record<string, unknown>) => invokeImpl<T>(cmd, args);

export const api = {
  openRepo: (path?: string) => call<Repo>("open_repo", { path: path ?? null }),
  initRepo: () => call<Repo>("init_repo"),
  trustRepo: () => call<Repo>("trust_repo"),
  overview: () => call<Overview>("overview"),
  markSeen: (seq: number) => call<void>("mark_seen", { seq }),
  task: (id: string) => call<TaskDetail>("task_detail", { id }),
  run: (id: string, after = 0) => call<RunDetail>("run_detail", { id, after }),
  evidenceLog: (id: number) => call<string>("evidence_log", { id }),
  review: (run: string) => call<Review>("review", { run }),
  fileDiff: (run: string, path: string) => call<FileDiff>("file_diff", { run, path }),
  startRun: (task: string, agent: string, policy: "ask" | "auto", note?: string, from?: string) =>
    call<string>("start_run", { task, agent, policy, note: note ?? null, from: from ?? null }),
  stopRun: (id: string) => call<void>("stop_run", { id }),
  answerAsk: (id: number, option: string) => call<boolean>("answer_ask", { id, option }),
  answerQuestion: (id: string, answer: string) => call<void>("answer_question", { id, answer }),
  accept: (run: string, closeTask: boolean, approval?: string) =>
    call<Accepted>("accept_run", { run, closeTask, approval: approval ?? null }),
  discard: (run: string) => call<string[]>("discard_run", { run }),
  newEntity: (kind: string, title: string, opts: { scope?: string[]; checks?: string[]; after?: string[]; blocks?: string[]; body?: string } = {}) =>
    call<{ id: string; path: string }>("new_entity", {
      kind,
      title,
      scope: opts.scope ?? [],
      checks: opts.checks ?? [],
      after: opts.after ?? [],
      blocks: opts.blocks ?? [],
      body: opts.body ?? null,
    }),
  rules: () => call<Rules>("rules"),
  runChecks: (names: string[] = []) => call<Evidence[]>("run_checks", { names }),
  readFile: (path: string) => call<FileText>("read_file", { path }),
  writeFile: (path: string, text: string, version: string | null) => call<string>("write_file", { path, text, version }),
  listFiles: () => call<string[]>("list_files"),
};

export const onChanged = (cb: () => void) => listenImpl("kitsu://changed", cb);

export function errorText(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

export function errorKind(e: unknown): string {
  if (e && typeof e === "object" && "kind" in e) return String((e as { kind: unknown }).kind);
  return "unknown";
}
