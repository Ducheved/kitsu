// The only way the UI talks to the system. In the desktop app this is
// Tauri IPC to the typed commands in src-tauri/src/commands.rs. Opened in a
// plain browser (vite dev, UI tests) it runs against fixture data instead,
// and the window says so.
//
// Every command that touches a repository names it. `repoApi(id)` binds the
// id once; a component takes its copy when it mounts (see `app.api`), so a
// reply or a follow-up call can't drift into another project after a switch.

import type {
  Accepted,
  FileDiff,
  FileText,
  Overview,
  Plan,
  Project,
  ProjectSummary,
  Repo,
  RepoTree,
  Review,
  Rules,
  RunDetail,
  TaskDetail,
  Evidence,
} from "./types";

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
type Listen = (event: string, cb: (payload: unknown) => void) => Promise<() => void>;

export const native = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let invokeImpl: Invoke;
let listenImpl: Listen;

if (native) {
  const core = await import("@tauri-apps/api/core");
  const ev = await import("@tauri-apps/api/event");
  invokeImpl = core.invoke;
  listenImpl = async (event, cb) => ev.listen(event, (e) => cb(e.payload));
} else {
  const mock = await import("./mock");
  invokeImpl = mock.invoke;
  listenImpl = mock.listen;
}

const call = <T>(cmd: string, args?: Record<string, unknown>) => invokeImpl<T>(cmd, args);

/** The workspaces list itself: the only commands without a repository. */
export const projects = {
  /** The project the app was started in, added to the list if needed. */
  launch: () => call<Project | null>("launch_repo"),
  list: () => call<Project[]>("list_workspaces"),
  overview: () => call<ProjectSummary[]>("workspace_overview"),
  add: (path: string, name?: string) => call<Project>("add_workspace", { path, name: name ?? null }),
  /** Only takes it off the list; nothing on disk changes. */
  remove: (repo: string) => call<Project>("remove_workspace", { repo }),
  rename: (repo: string, name: string | null) => call<Project>("rename_workspace", { repo, name }),
  move: (repo: string, index: number) => call<void>("move_workspace", { repo, index }),
};

/** Every command about one repository, bound to its id. */
export function repoApi(repo: string) {
  const at = <T>(cmd: string, args: Record<string, unknown> = {}) => call<T>(cmd, { ...args, repo });
  return {
    repo,
    openRepo: () => at<Repo>("open_repo"),
    initRepo: () => at<Repo>("init_repo"),
    trustRepo: () => at<Repo>("trust_repo"),
    overview: () => at<Overview>("overview"),
    markSeen: (seq: number) => at<void>("mark_seen", { seq }),
    task: (id: string) => at<TaskDetail>("task_detail", { id }),
    run: (id: string, after = 0) => at<RunDetail>("run_detail", { id, after }),
    evidenceLog: (id: number) => at<string>("evidence_log", { id }),
    review: (run: string) => at<Review>("review", { run }),
    fileDiff: (run: string, path: string) => at<FileDiff>("file_diff", { run, path }),
    startRun: (task: string, agent: string, policy: "ask" | "auto", note?: string, from?: string) =>
      at<string>("start_run", { task, agent, policy, note: note ?? null, from: from ?? null }),
    stopRun: (id: string) => at<void>("stop_run", { id }),
    answerAsk: (id: number, option: string) => at<boolean>("answer_ask", { id, option }),
    answerQuestion: (id: string, answer: string) => at<void>("answer_question", { id, answer }),
    accept: (run: string, closeTask: boolean, approval?: string) =>
      at<Accepted>("accept_run", { run, closeTask, approval: approval ?? null }),
    discard: (run: string) => at<string[]>("discard_run", { run }),
    newEntity: (kind: string, title: string, opts: { scope?: string[]; checks?: string[]; after?: string[]; blocks?: string[]; body?: string } = {}) =>
      at<{ id: string; path: string }>("new_entity", {
        kind,
        title,
        scope: opts.scope ?? [],
        checks: opts.checks ?? [],
        after: opts.after ?? [],
        blocks: opts.blocks ?? [],
        body: opts.body ?? null,
      }),
    plan: () => at<Plan>("plan"),
    /** Rewrites only the given lists in the task's front matter; returns the file's new version. */
    updateTask: (id: string, lists: { after?: string[]; checks?: string[]; scope?: string[] }, version?: string) =>
      at<string>("update_task", { id, after: lists.after ?? null, checks: lists.checks ?? null, scope: lists.scope ?? null, version: version ?? null }),
    rules: () => at<Rules>("rules"),
    runChecks: (names: string[] = []) => at<Evidence[]>("run_checks", { names }),
    readFile: (path: string) => at<FileText>("read_file", { path }),
    writeFile: (path: string, text: string, version: string | null) => at<string>("write_file", { path, text, version }),
    listFiles: () => at<string[]>("list_files"),
    tree: () => at<RepoTree>("git_view"),
  };
}

export type RepoApi = ReturnType<typeof repoApi>;

/** `repo` is the project whose state moved; null means "maybe any". */
export const onChanged = (cb: (repo: string | null) => void) =>
  listenImpl("kitsu://changed", (p) => {
    const repo = p && typeof p === "object" && "repo" in p ? String((p as { repo: unknown }).repo) : null;
    cb(repo);
  });

export function errorText(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

export function errorKind(e: unknown): string {
  if (e && typeof e === "object" && "kind" in e) return String((e as { kind: unknown }).kind);
  return "unknown";
}
