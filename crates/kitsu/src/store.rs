//! Execution state: runs, evidence, events, pending questions from live
//! agents, integrations.
//!
//! One SQLite file per repository clone, in the git common dir. Several
//! processes use it at once (the app, `kitsu run` workers, the CLI), so every
//! write is a short transaction and WAL mode lets readers proceed while a
//! writer commits.
//!
//! What is *not* here: tasks, checks, decisions and questions (those are
//! files in the repo), and anything written only because time passed. There
//! are no heartbeats. Liveness comes from OS file locks, see `workspace.rs`.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::run::{Outcome, RunEvent, RunState, reduce};
use crate::util::now_ms;

const SCHEMA_VERSION: i64 = 3;

/// v3. Typed judgments (`judge.rs`): what was asked about which inputs (by
/// hash), what came back, or why nothing did. One row per request.
const JUDGMENTS: &str = r#"
CREATE TABLE IF NOT EXISTS judgments (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    run           TEXT,
    purpose       TEXT NOT NULL,
    kind          TEXT NOT NULL,
    inputs        TEXT NOT NULL,
    outcome       TEXT NOT NULL,
    answers       TEXT NOT NULL,
    reason        TEXT,
    model         TEXT,
    latency_ms    INTEGER NOT NULL,
    attempts      INTEGER NOT NULL,
    input_tokens  INTEGER,
    output_tokens INTEGER,
    request_id    TEXT,
    created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS judgments_by_run ON judgments(run, id);
"#;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS runs (
    id               TEXT PRIMARY KEY,
    task             TEXT NOT NULL,
    agent            TEXT NOT NULL,
    base             TEXT NOT NULL,
    branch           TEXT NOT NULL,
    worktree         TEXT NOT NULL,
    state            TEXT NOT NULL,
    stop_reason      TEXT,
    detail           TEXT,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    owner            TEXT,
    pid              INTEGER,
    brief            TEXT,
    snapshot         TEXT,
    resolution       TEXT,
    from_run         TEXT,
    note             TEXT,
    created_at       INTEGER NOT NULL,
    ended_at         INTEGER,
    -- Fixed once the snapshot exists: base..snapshot never changes, so
    -- status refreshes read these instead of asking git every time.
    snapshot_tree    TEXT,
    changed          TEXT,
    -- What the agent reported about tokens and cost (JSON), if anything.
    usage            TEXT
);
CREATE INDEX IF NOT EXISTS runs_by_task ON runs(task, created_at);
CREATE INDEX IF NOT EXISTS runs_live ON runs(state) WHERE state IN ('starting', 'running', 'stopping');
CREATE INDEX IF NOT EXISTS runs_unresolved ON runs(created_at) WHERE resolution IS NULL;

CREATE TABLE IF NOT EXISTS events (
    seq  INTEGER PRIMARY KEY AUTOINCREMENT,
    run  TEXT,
    at   INTEGER NOT NULL,
    kind TEXT NOT NULL,
    body TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS events_by_run ON events(run, seq);

CREATE TABLE IF NOT EXISTS evidence (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    check_name  TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    command     TEXT NOT NULL,
    tree        TEXT NOT NULL,
    tree_after  TEXT,
    outcome     TEXT NOT NULL,
    exit_code   INTEGER,
    duration_ms INTEGER NOT NULL,
    log         TEXT,
    log_bytes   INTEGER,
    run         TEXT,
    started_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS evidence_by_tree ON evidence(fingerprint, tree, id);
CREATE INDEX IF NOT EXISTS evidence_by_check ON evidence(check_name, id);

CREATE TABLE IF NOT EXISTS asks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    run         TEXT NOT NULL,
    request     TEXT NOT NULL,
    answer      TEXT,
    created_at  INTEGER NOT NULL,
    answered_at INTEGER
);
CREATE INDEX IF NOT EXISTS asks_open ON asks(run) WHERE answer IS NULL;

CREATE TABLE IF NOT EXISTS integrations (
    id         TEXT PRIMARY KEY,
    run        TEXT NOT NULL,
    target     TEXT NOT NULL,
    expected   TEXT NOT NULL,
    candidate  TEXT,
    state      TEXT NOT NULL,
    detail     TEXT,
    close_task INTEGER NOT NULL,
    owner      TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    ended_at   INTEGER
);
CREATE INDEX IF NOT EXISTS integrations_by_run ON integrations(run);
"#;

#[derive(Debug, Clone, Serialize)]
pub struct RunRow {
    pub id: String,
    pub task: String,
    pub agent: String,
    pub base: String,
    pub branch: String,
    pub worktree: String,
    pub state: RunState,
    pub stop_reason: Option<String>,
    pub detail: Option<String>,
    pub cancel_requested: bool,
    pub owner: Option<String>,
    pub pid: Option<u32>,
    pub brief: Option<String>,
    pub snapshot: Option<String>,
    pub resolution: Option<String>,
    pub from_run: Option<String>,
    pub note: Option<String>,
    pub created_at: i64,
    pub ended_at: Option<i64>,
    pub snapshot_tree: Option<String>,
    /// Paths changed between `base` and `snapshot`.
    pub changed: Option<Vec<String>>,
    /// Tokens and cost as the agent reported them. None when it reported
    /// nothing, which is common: ACP makes this optional.
    pub usage: Option<Usage>,
}

/// Token accounting for one run, in the agent's own numbers. Every field is
/// optional because agents report different subsets; a missing field means
/// "not reported", never zero.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_write: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Tokens in the context window at the last report, and its size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

impl Usage {
    /// Fold in a `usage_update` session notification: context occupancy and
    /// cumulative cost.
    pub fn apply_update(&mut self, u: &Value) {
        if let Some(n) = u["used"].as_u64() {
            self.context_used = Some(n);
        }
        if let Some(n) = u["size"].as_u64() {
            self.context_size = Some(n);
        }
        if let (Some(a), Some(c)) = (u["cost"]["amount"].as_f64(), u["cost"]["currency"].as_str()) {
            self.cost = Some(a);
            self.currency = Some(c.to_string());
        }
    }

    /// Fold in the `usage` object of a `session/prompt` response (cumulative
    /// token counts for the session).
    pub fn apply_turn(&mut self, u: &Value) {
        let get = |k: &str| u[k].as_u64();
        self.input = get("inputTokens").or(self.input);
        self.output = get("outputTokens").or(self.output);
        self.cached_read = get("cachedReadTokens").or(self.cached_read);
        self.cached_write = get("cachedWriteTokens").or(self.cached_write);
        self.thought = get("thoughtTokens").or(self.thought);
        self.total = get("totalTokens").or(self.total);
    }

    pub fn is_empty(&self) -> bool {
        *self == Usage::default()
    }

    /// Tokens the run spent, as far as the agent said: total if given,
    /// otherwise input + output.
    pub fn spent(&self) -> Option<u64> {
        self.total.or(match (self.input, self.output) {
            (None, None) => None,
            (i, o) => Some(i.unwrap_or(0) + o.unwrap_or(0)),
        })
    }
}

#[derive(Debug, Clone)]
pub struct NewRun<'a> {
    pub id: &'a str,
    pub task: &'a str,
    pub agent: &'a str,
    pub base: &'a str,
    pub branch: &'a str,
    pub worktree: &'a str,
    pub owner: &'a str,
    pub from_run: Option<&'a str>,
    pub note: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventRow {
    pub seq: i64,
    pub run: Option<String>,
    pub at: i64,
    pub kind: String,
    pub body: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Pass,
    Fail,
    /// Ran past its timeout and was killed.
    Timeout,
    /// Could not produce a result at all (command missing, spawn failure).
    Error,
}

impl CheckOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckOutcome::Pass => "pass",
            CheckOutcome::Fail => "fail",
            CheckOutcome::Timeout => "timeout",
            CheckOutcome::Error => "error",
        }
    }

    fn parse(s: &str) -> CheckOutcome {
        match s {
            "pass" => CheckOutcome::Pass,
            "fail" => CheckOutcome::Fail,
            "timeout" => CheckOutcome::Timeout,
            _ => CheckOutcome::Error,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceRow {
    pub id: i64,
    pub check_name: String,
    pub fingerprint: String,
    pub command: String,
    pub tree: String,
    /// If this differs from `tree`, files changed while the check ran and
    /// the result is not bound to either tree.
    pub tree_after: Option<String>,
    pub outcome: CheckOutcome,
    pub exit_code: Option<i32>,
    pub duration_ms: i64,
    pub log: Option<String>,
    pub log_bytes: Option<i64>,
    pub run: Option<String>,
    pub started_at: i64,
}

impl EvidenceRow {
    /// Only evidence whose tree did not move during the check says anything
    /// about a specific tree.
    pub fn is_bound(&self) -> bool {
        self.tree_after.as_deref().is_none_or(|a| a == self.tree)
    }
}

#[derive(Debug, Clone)]
pub struct NewEvidence<'a> {
    pub check_name: &'a str,
    pub fingerprint: &'a str,
    pub command: &'a str,
    pub tree: &'a str,
    pub tree_after: Option<&'a str>,
    pub outcome: CheckOutcome,
    pub exit_code: Option<i32>,
    pub duration_ms: i64,
    pub log: Option<&'a str>,
    pub log_bytes: Option<i64>,
    pub run: Option<&'a str>,
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AskRow {
    pub id: i64,
    pub run: String,
    pub request: Value,
    pub answer: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewJudgment<'a> {
    pub run: Option<&'a str>,
    /// Which caller asked: `permission`, later `rerank`, `guard`, ...
    pub purpose: &'a str,
    /// Question kinds in order, comma-separated (`yes_no`, `choice`, `score`).
    pub kind: &'a str,
    /// SHA-256 of the request body (without the key).
    pub inputs: &'a str,
    /// `answered` when every question got a valid answer, else `unknown`.
    pub outcome: &'a str,
    /// By question name: the answer and its probabilities, or why none.
    pub answers: &'a Value,
    pub reason: Option<&'a str>,
    pub model: Option<&'a str>,
    pub latency_ms: i64,
    pub attempts: i64,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub request_id: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JudgmentRow {
    pub id: i64,
    pub run: Option<String>,
    pub purpose: String,
    pub kind: String,
    pub inputs: String,
    pub outcome: String,
    pub answers: Value,
    pub reason: Option<String>,
    pub model: Option<String>,
    pub latency_ms: i64,
    pub attempts: i64,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub request_id: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationState {
    Preparing,
    Verifying,
    /// The ref update may or may not have happened. Recovery asks git.
    Applying,
    Applied,
    Conflict,
    Rejected,
    Failed,
    Abandoned,
}

impl IntegrationState {
    pub fn as_str(self) -> &'static str {
        match self {
            IntegrationState::Preparing => "preparing",
            IntegrationState::Verifying => "verifying",
            IntegrationState::Applying => "applying",
            IntegrationState::Applied => "applied",
            IntegrationState::Conflict => "conflict",
            IntegrationState::Rejected => "rejected",
            IntegrationState::Failed => "failed",
            IntegrationState::Abandoned => "abandoned",
        }
    }

    fn parse(s: &str) -> IntegrationState {
        match s {
            "preparing" => IntegrationState::Preparing,
            "verifying" => IntegrationState::Verifying,
            "applying" => IntegrationState::Applying,
            "applied" => IntegrationState::Applied,
            "conflict" => IntegrationState::Conflict,
            "rejected" => IntegrationState::Rejected,
            "failed" => IntegrationState::Failed,
            _ => IntegrationState::Abandoned,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(
            self,
            IntegrationState::Preparing | IntegrationState::Verifying | IntegrationState::Applying
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationRow {
    pub id: String,
    pub run: String,
    pub target: String,
    pub expected: String,
    pub candidate: Option<String>,
    pub state: IntegrationState,
    pub detail: Option<String>,
    pub close_task: bool,
    pub owner: String,
    pub created_at: i64,
    pub ended_at: Option<i64>,
}

pub struct Store {
    conn: Connection,
}

/// What `apply_run_event` did, so callers can react (e.g. print a warning on
/// a duplicate completion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    Moved(RunState),
    Unchanged,
    Duplicate,
}

impl Store {
    /// Open (and create or migrate) the state database.
    ///
    /// Opening is idempotent, so it is retried a few times on SQLITE_BUSY:
    /// some of the locks taken while opening a WAL database don't go
    /// through the busy handler, and with hundreds of workers starting at
    /// once one of them occasionally loses. Five attempts over about two
    /// seconds, then the error is reported as is.
    pub fn open(path: &Path) -> Result<Store> {
        let mut delay = std::time::Duration::from_millis(40);
        for attempt in 1.. {
            match Store::open_once(path) {
                Err(Error::Db(rusqlite::Error::SqliteFailure(e, _)))
                    if e.code == rusqlite::ErrorCode::DatabaseBusy && attempt < 5 =>
                {
                    std::thread::sleep(delay);
                    delay *= 2;
                }
                other => return other,
            }
        }
        unreachable!("the loop returns")
    }

    fn open_once(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        let mode: String = conn.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            conn.pragma_update(None, "journal_mode", "WAL")?;
        }
        // NORMAL in WAL mode survives process crashes; a power loss can drop
        // the last transactions. Every effect Kitsu performs can be
        // reconciled against git or the filesystem, see docs/design.md.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Store> {
        let store = Store {
            conn: Connection::open_in_memory()?,
        };
        store.migrate()?;
        Ok(store)
    }

    /// Every write goes through `BEGIN IMMEDIATE`. A deferred transaction
    /// that reads and then writes gets SQLITE_BUSY straight away, without
    /// honoring busy_timeout, when another process committed in between;
    /// with a few hundred workers that happens constantly. Found by the
    /// 300-run scale probe.
    fn write_tx(&self) -> Result<rusqlite::Transaction<'_>> {
        Ok(rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::Invalid(format!(
                "state database is schema v{version}, this build understands v{SCHEMA_VERSION}; upgrade kitsu"
            )));
        }
        if version < SCHEMA_VERSION {
            // Two processes can open an old database at once; the write lock
            // makes one of them migrate and the other see the result.
            let tx = self.write_tx()?;
            let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
            if version == 0 {
                tx.execute_batch(SCHEMA)?;
            }
            if version == 1 {
                tx.execute_batch("ALTER TABLE runs ADD COLUMN usage TEXT;")?;
            }
            if version < 3 {
                tx.execute_batch(JUDGMENTS)?;
            }
            if version < SCHEMA_VERSION {
                tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            tx.commit()?;
        }
        Ok(())
    }

    /// Changes whenever *another* connection commits. Pollers use this to
    /// skip queries when nothing happened.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "data_version", |r| r.get(0))?)
    }

    // ---- runs --------------------------------------------------------------

    pub fn insert_run(&self, r: &NewRun<'_>) -> Result<()> {
        let tx = self.write_tx()?;
        tx.execute(
            "INSERT INTO runs (id, task, agent, base, branch, worktree, state, owner, from_run, note, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'starting', ?7, ?8, ?9, ?10)",
            params![r.id, r.task, r.agent, r.base, r.branch, r.worktree, r.owner, r.from_run, r.note, now_ms()],
        )?;
        append_event(
            &tx,
            Some(r.id),
            "run.created",
            // The version lets recovery and review tell which rules (reducer,
            // brief compiler) a run was started under.
            &serde_json::json!({ "task": r.task, "agent": r.agent, "base": r.base, "kitsu_version": env!("CARGO_PKG_VERSION") }),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn run(&self, id: &str) -> Result<RunRow> {
        self.conn
            .query_row(
                &format!("SELECT {RUN_COLS} FROM runs WHERE id = ?1"),
                [id],
                run_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("run {id}")))
    }

    pub fn runs_for_task(&self, task: &str) -> Result<Vec<RunRow>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {RUN_COLS} FROM runs WHERE task = ?1 ORDER BY created_at DESC"
        ))?;
        let rows = stmt
            .query_map([task], run_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Runs nobody has accepted or discarded, newest first.
    pub fn unresolved_runs(&self) -> Result<Vec<RunRow>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {RUN_COLS} FROM runs WHERE resolution IS NULL ORDER BY created_at DESC, id"
        ))?;
        let rows = stmt
            .query_map([], run_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn live_runs(&self) -> Result<Vec<RunRow>> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!("SELECT {RUN_COLS} FROM runs WHERE state IN ('starting', 'running', 'stopping') ORDER BY created_at"))?;
        let rows = stmt
            .query_map([], run_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn recent_runs(&self, limit: usize) -> Result<Vec<RunRow>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {RUN_COLS} FROM runs ORDER BY created_at DESC LIMIT ?1"
        ))?;
        let rows = stmt
            .query_map([limit as i64], run_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The only way run state changes. Reads, reduces and writes in one
    /// transaction, so two processes racing on the same run cannot both win.
    /// Differences between a run's stored state and its recorded
    /// transitions: each `run.state` must start where the previous one
    /// ended (from `starting`), and the last must end at the stored state.
    /// Empty when consistent. A state written without going through
    /// `apply_run_event` shows up here.
    pub fn run_history_mismatches(&self, id: &str) -> Result<Vec<String>> {
        let run = self.run(id)?;
        let mut at = RunState::Starting.as_str().to_string();
        let mut out = Vec::new();
        for e in self.run_events(id, 0, usize::MAX)? {
            if e.kind != "run.state" {
                continue;
            }
            let from = e.body["from"].as_str().unwrap_or("?");
            let to = e.body["to"].as_str().unwrap_or("?");
            if from != at {
                out.push(format!(
                    "event {} moves from {from}, but the run was {at}",
                    e.seq
                ));
            }
            at = to.to_string();
        }
        if at != run.state.as_str() {
            out.push(format!(
                "stored state is {}, events end at {at}",
                run.state.as_str()
            ));
        }
        Ok(out)
    }

    pub fn apply_run_event(&self, id: &str, event: &RunEvent) -> Result<Applied> {
        let tx = self.write_tx()?;
        let state: String = tx
            .query_row("SELECT state FROM runs WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("run {id}")))?;
        let from = RunState::parse(&state)
            .ok_or_else(|| Error::Invalid(format!("run {id} has unknown state {state}")))?;
        let outcome = reduce(from, event).map_err(|e| Error::Invalid(e.to_string()))?;
        let body = serde_json::json!({ "event": format!("{event:?}"), "from": from.as_str() });
        let applied = match outcome {
            Outcome::Move {
                to,
                stop_reason,
                detail,
            } => {
                let ended = to.is_terminal().then(now_ms);
                tx.execute(
                    "UPDATE runs SET state = ?2,
                        stop_reason = COALESCE(?3, stop_reason),
                        detail = COALESCE(?4, detail),
                        ended_at = COALESCE(?5, ended_at),
                        cancel_requested = cancel_requested OR ?6
                     WHERE id = ?1",
                    params![
                        id,
                        to.as_str(),
                        stop_reason,
                        detail,
                        ended,
                        matches!(event, RunEvent::CancelRequested)
                    ],
                )?;
                let mut b = body;
                b["to"] = to.as_str().into();
                if let Some(r) = &stop_reason {
                    b["stop_reason"] = r.clone().into();
                }
                if let Some(d) = &detail {
                    b["detail"] = d.clone().into();
                }
                append_event(&tx, Some(id), "run.state", &b)?;
                Applied::Moved(to)
            }
            Outcome::Same => {
                if matches!(event, RunEvent::CancelRequested) {
                    tx.execute("UPDATE runs SET cancel_requested = 1 WHERE id = ?1", [id])?;
                }
                Applied::Unchanged
            }
            Outcome::Duplicate => {
                append_event(&tx, Some(id), "run.duplicate", &body)?;
                Applied::Duplicate
            }
        };
        tx.commit()?;
        Ok(applied)
    }

    pub fn set_run_pid(&self, id: &str, pid: Option<u32>) -> Result<()> {
        self.conn
            .execute("UPDATE runs SET pid = ?2 WHERE id = ?1", params![id, pid])?;
        Ok(())
    }

    pub fn set_run_usage(&self, id: &str, usage: &Usage) -> Result<()> {
        let json = serde_json::to_string(usage).unwrap_or_else(|_| "{}".into());
        self.conn.execute(
            "UPDATE runs SET usage = ?2 WHERE id = ?1",
            params![id, json],
        )?;
        Ok(())
    }

    pub fn set_run_brief(&self, id: &str, blob: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET brief = ?2 WHERE id = ?1",
            params![id, blob],
        )?;
        Ok(())
    }

    pub fn set_run_snapshot(
        &self,
        id: &str,
        commit: &str,
        tree: &str,
        changed: &[String],
    ) -> Result<()> {
        let tx = self.write_tx()?;
        let list = serde_json::to_string(changed).unwrap_or_else(|_| "[]".into());
        tx.execute(
            "UPDATE runs SET snapshot = ?2, snapshot_tree = ?3, changed = ?4 WHERE id = ?1",
            params![id, commit, tree, list],
        )?;
        append_event(
            &tx,
            Some(id),
            "run.snapshot",
            &serde_json::json!({ "commit": commit, "files": changed.len() }),
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Accept or discard, once. A second resolution is a conflict: somebody
    /// else already decided.
    pub fn resolve_run(&self, id: &str, resolution: &str) -> Result<()> {
        let tx = self.write_tx()?;
        let n = tx.execute(
            "UPDATE runs SET resolution = ?2 WHERE id = ?1 AND resolution IS NULL",
            params![id, resolution],
        )?;
        if n == 0 {
            let current: Option<String> = tx
                .query_row("SELECT resolution FROM runs WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?
                .flatten();
            return Err(match current {
                Some(c) => Error::Conflict(format!("run {id} is already {c}")),
                None => Error::NotFound(format!("run {id}")),
            });
        }
        append_event(
            &tx,
            Some(id),
            "run.resolved",
            &serde_json::json!({ "resolution": resolution }),
        )?;
        tx.commit()?;
        Ok(())
    }

    // ---- events ------------------------------------------------------------

    pub fn append(&self, run: Option<&str>, kind: &str, body: &Value) -> Result<i64> {
        append_event(&self.conn, run, kind, body)
    }

    /// Several events in one transaction. The run worker flushes agent
    /// updates this way instead of one commit per chunk.
    pub fn append_batch(&self, run: &str, events: &[(String, Value)]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let tx = self.write_tx()?;
        for (kind, body) in events {
            append_event(&tx, Some(run), kind, body)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn events_after(&self, seq: i64, limit: usize) -> Result<Vec<EventRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT seq, run, at, kind, body FROM events WHERE seq > ?1 ORDER BY seq LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![seq, limit as i64], event_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn run_events(&self, run: &str, after: i64, limit: usize) -> Result<Vec<EventRow>> {
        let mut stmt =
            self.conn.prepare_cached("SELECT seq, run, at, kind, body FROM events WHERE run = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3")?;
        let rows = stmt
            .query_map(params![run, after, limit as i64], event_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The latest event of `kind` in `run`, if any.
    pub fn last_run_event(&self, run: &str, kind: &str) -> Result<Option<EventRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT seq, run, at, kind, body FROM events WHERE run = ?1 AND kind = ?2 ORDER BY seq DESC LIMIT 1",
                params![run, kind],
                event_row,
            )
            .optional()?)
    }

    pub fn last_seq(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |r| r.get(0))?)
    }

    // ---- evidence ----------------------------------------------------------

    pub fn insert_evidence(&self, e: &NewEvidence<'_>) -> Result<i64> {
        let tx = self.write_tx()?;
        tx.execute(
            "INSERT INTO evidence (check_name, fingerprint, command, tree, tree_after, outcome, exit_code,
                                   duration_ms, log, log_bytes, run, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                e.check_name,
                e.fingerprint,
                e.command,
                e.tree,
                e.tree_after,
                e.outcome.as_str(),
                e.exit_code,
                e.duration_ms,
                e.log,
                e.log_bytes,
                e.run,
                e.started_at
            ],
        )?;
        let id = tx.last_insert_rowid();
        append_event(
            &tx,
            e.run,
            "check.done",
            &serde_json::json!({ "evidence": id, "check": e.check_name, "outcome": e.outcome.as_str(), "tree": e.tree }),
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Latest bound evidence for this exact check definition on this tree.
    pub fn evidence_at(&self, fingerprint: &str, tree: &str) -> Result<Option<EvidenceRow>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {EVIDENCE_COLS} FROM evidence
                     WHERE fingerprint = ?1 AND tree = ?2 AND (tree_after IS NULL OR tree_after = tree)
                     ORDER BY id DESC LIMIT 1"
                ),
                params![fingerprint, tree],
                evidence_row,
            )
            .optional()?)
    }

    /// Most recent bound evidence for this check definition on any tree.
    /// Used to report "stale: last passed on another tree".
    pub fn latest_evidence(&self, fingerprint: &str) -> Result<Option<EvidenceRow>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {EVIDENCE_COLS} FROM evidence
                     WHERE fingerprint = ?1 AND (tree_after IS NULL OR tree_after = tree)
                     ORDER BY id DESC LIMIT 1"
                ),
                [fingerprint],
                evidence_row,
            )
            .optional()?)
    }

    pub fn evidence(&self, id: i64) -> Result<EvidenceRow> {
        self.conn
            .query_row(
                &format!("SELECT {EVIDENCE_COLS} FROM evidence WHERE id = ?1"),
                [id],
                evidence_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("evidence {id}")))
    }

    pub fn evidence_for_run(&self, run: &str) -> Result<Vec<EvidenceRow>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {EVIDENCE_COLS} FROM evidence WHERE run = ?1 ORDER BY id"
        ))?;
        let rows = stmt
            .query_map([run], evidence_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---- judgments ---------------------------------------------------------

    pub fn insert_judgment(&self, j: &NewJudgment<'_>) -> Result<i64> {
        let tx = self.write_tx()?;
        tx.execute(
            "INSERT INTO judgments (run, purpose, kind, inputs, outcome, answers, reason, model, latency_ms,
                                    attempts, input_tokens, output_tokens, request_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                j.run,
                j.purpose,
                j.kind,
                j.inputs,
                j.outcome,
                j.answers.to_string(),
                j.reason,
                j.model,
                j.latency_ms,
                j.attempts,
                j.input_tokens,
                j.output_tokens,
                j.request_id,
                now_ms()
            ],
        )?;
        let id = tx.last_insert_rowid();
        append_event(
            &tx,
            j.run,
            "judge.done",
            &serde_json::json!({ "judgment": id, "purpose": j.purpose, "outcome": j.outcome, "reason": j.reason }),
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn judgment(&self, id: i64) -> Result<JudgmentRow> {
        self.conn
            .query_row(
                &format!("SELECT {JUDGMENT_COLS} FROM judgments WHERE id = ?1"),
                [id],
                judgment_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("judgment {id}")))
    }

    pub fn judgments_for_run(&self, run: &str) -> Result<Vec<JudgmentRow>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {JUDGMENT_COLS} FROM judgments WHERE run = ?1 ORDER BY id"
        ))?;
        let rows = stmt
            .query_map([run], judgment_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---- asks: a live agent waiting on a human ------------------------------

    pub fn insert_ask(&self, run: &str, request: &Value) -> Result<i64> {
        let tx = self.write_tx()?;
        tx.execute(
            "INSERT INTO asks (run, request, created_at) VALUES (?1, ?2, ?3)",
            params![run, request.to_string(), now_ms()],
        )?;
        let id = tx.last_insert_rowid();
        append_event(
            &tx,
            Some(run),
            "ask.open",
            &serde_json::json!({ "ask": id, "request": request }),
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// First answer wins. Returns false if it was already answered.
    pub fn answer_ask(&self, id: i64, answer: &str) -> Result<bool> {
        let tx = self.write_tx()?;
        let n = tx.execute(
            "UPDATE asks SET answer = ?2, answered_at = ?3 WHERE id = ?1 AND answer IS NULL",
            params![id, answer, now_ms()],
        )?;
        if n == 1 {
            let run: String =
                tx.query_row("SELECT run FROM asks WHERE id = ?1", [id], |r| r.get(0))?;
            append_event(
                &tx,
                Some(&run),
                "ask.answered",
                &serde_json::json!({ "ask": id, "answer": answer }),
            )?;
        }
        tx.commit()?;
        Ok(n == 1)
    }

    pub fn ask(&self, id: i64) -> Result<AskRow> {
        self.conn
            .query_row(
                "SELECT id, run, request, answer, created_at FROM asks WHERE id = ?1",
                [id],
                ask_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("ask {id}")))
    }

    pub fn open_asks(&self) -> Result<Vec<AskRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT a.id, a.run, a.request, a.answer, a.created_at FROM asks a JOIN runs r ON r.id = a.run
             WHERE a.answer IS NULL AND r.state IN ('starting', 'running', 'stopping') ORDER BY a.id",
        )?;
        let rows = stmt
            .query_map([], ask_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---- integrations ------------------------------------------------------

    pub fn insert_integration(
        &self,
        id: &str,
        run: &str,
        target: &str,
        expected: &str,
        close_task: bool,
        owner: &str,
    ) -> Result<()> {
        let tx = self.write_tx()?;
        tx.execute(
            "INSERT INTO integrations (id, run, target, expected, state, close_task, owner, created_at)
             VALUES (?1, ?2, ?3, ?4, 'preparing', ?5, ?6, ?7)",
            params![id, run, target, expected, close_task, owner, now_ms()],
        )?;
        append_event(
            &tx,
            Some(run),
            "integration.started",
            &serde_json::json!({ "integration": id, "target": target, "expected": expected }),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_integration(
        &self,
        id: &str,
        state: IntegrationState,
        candidate: Option<&str>,
        detail: Option<&str>,
    ) -> Result<()> {
        let tx = self.write_tx()?;
        let ended = state.is_terminal().then(now_ms);
        let n = tx.execute(
            "UPDATE integrations SET state = ?2, candidate = COALESCE(?3, candidate), detail = COALESCE(?4, detail),
                    ended_at = COALESCE(?5, ended_at)
             WHERE id = ?1 AND state IN ('preparing', 'verifying', 'applying')",
            params![id, state.as_str(), candidate, detail, ended],
        )?;
        if n == 0 {
            return Err(Error::Conflict(format!(
                "integration {id} already finished"
            )));
        }
        let run: String =
            tx.query_row("SELECT run FROM integrations WHERE id = ?1", [id], |r| {
                r.get(0)
            })?;
        append_event(
            &tx,
            Some(&run),
            "integration.state",
            &serde_json::json!({ "integration": id, "state": state.as_str(), "candidate": candidate, "detail": detail }),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn integration(&self, id: &str) -> Result<IntegrationRow> {
        self.conn
            .query_row(
                &format!("SELECT {INTEGRATION_COLS} FROM integrations WHERE id = ?1"),
                [id],
                integration_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("integration {id}")))
    }

    pub fn unfinished_integrations(&self) -> Result<Vec<IntegrationRow>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {INTEGRATION_COLS} FROM integrations WHERE state IN ('preparing', 'verifying', 'applying') ORDER BY created_at"
        ))?;
        let rows = stmt
            .query_map([], integration_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn integrations_for_run(&self, run: &str) -> Result<Vec<IntegrationRow>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {INTEGRATION_COLS} FROM integrations WHERE run = ?1 ORDER BY created_at DESC"
        ))?;
        let rows = stmt
            .query_map([run], integration_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---- meta --------------------------------------------------------------

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value
             WHERE value IS NOT excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

fn append_event(conn: &Connection, run: Option<&str>, kind: &str, body: &Value) -> Result<i64> {
    conn.prepare_cached("INSERT INTO events (run, at, kind, body) VALUES (?1, ?2, ?3, ?4)")?
        .execute(params![run, now_ms(), kind, body.to_string()])?;
    Ok(conn.last_insert_rowid())
}

const RUN_COLS: &str = "id, task, agent, base, branch, worktree, state, stop_reason, detail, cancel_requested, owner, pid, brief, snapshot, resolution, from_run, note, created_at, ended_at, snapshot_tree, changed, usage";

fn run_row(r: &Row<'_>) -> rusqlite::Result<RunRow> {
    let state: String = r.get(6)?;
    Ok(RunRow {
        id: r.get(0)?,
        task: r.get(1)?,
        agent: r.get(2)?,
        base: r.get(3)?,
        branch: r.get(4)?,
        worktree: r.get(5)?,
        state: RunState::parse(&state).ok_or_else(|| {
            rusqlite::Error::InvalidColumnType(6, "state".into(), rusqlite::types::Type::Text)
        })?,
        stop_reason: r.get(7)?,
        detail: r.get(8)?,
        cancel_requested: r.get(9)?,
        owner: r.get(10)?,
        pid: r.get(11)?,
        brief: r.get(12)?,
        snapshot: r.get(13)?,
        resolution: r.get(14)?,
        from_run: r.get(15)?,
        note: r.get(16)?,
        created_at: r.get(17)?,
        ended_at: r.get(18)?,
        snapshot_tree: r.get(19)?,
        changed: r
            .get::<_, Option<String>>(20)?
            .and_then(|c| serde_json::from_str(&c).ok()),
        usage: r
            .get::<_, Option<String>>(21)?
            .and_then(|c| serde_json::from_str(&c).ok()),
    })
}

fn event_row(r: &Row<'_>) -> rusqlite::Result<EventRow> {
    let body: String = r.get(4)?;
    Ok(EventRow {
        seq: r.get(0)?,
        run: r.get(1)?,
        at: r.get(2)?,
        kind: r.get(3)?,
        body: serde_json::from_str(&body).unwrap_or(Value::String(body)),
    })
}

const EVIDENCE_COLS: &str = "id, check_name, fingerprint, command, tree, tree_after, outcome, exit_code, duration_ms, log, log_bytes, run, started_at";

fn evidence_row(r: &Row<'_>) -> rusqlite::Result<EvidenceRow> {
    let outcome: String = r.get(6)?;
    Ok(EvidenceRow {
        id: r.get(0)?,
        check_name: r.get(1)?,
        fingerprint: r.get(2)?,
        command: r.get(3)?,
        tree: r.get(4)?,
        tree_after: r.get(5)?,
        outcome: CheckOutcome::parse(&outcome),
        exit_code: r.get(7)?,
        duration_ms: r.get(8)?,
        log: r.get(9)?,
        log_bytes: r.get(10)?,
        run: r.get(11)?,
        started_at: r.get(12)?,
    })
}

const JUDGMENT_COLS: &str = "id, run, purpose, kind, inputs, outcome, answers, reason, model, latency_ms, attempts, input_tokens, output_tokens, request_id, created_at";

fn judgment_row(r: &Row<'_>) -> rusqlite::Result<JudgmentRow> {
    let answers: String = r.get(6)?;
    Ok(JudgmentRow {
        id: r.get(0)?,
        run: r.get(1)?,
        purpose: r.get(2)?,
        kind: r.get(3)?,
        inputs: r.get(4)?,
        outcome: r.get(5)?,
        answers: serde_json::from_str(&answers).unwrap_or(Value::String(answers)),
        reason: r.get(7)?,
        model: r.get(8)?,
        latency_ms: r.get(9)?,
        attempts: r.get(10)?,
        input_tokens: r.get(11)?,
        output_tokens: r.get(12)?,
        request_id: r.get(13)?,
        created_at: r.get(14)?,
    })
}

fn ask_row(r: &Row<'_>) -> rusqlite::Result<AskRow> {
    let req: String = r.get(2)?;
    Ok(AskRow {
        id: r.get(0)?,
        run: r.get(1)?,
        request: serde_json::from_str(&req).unwrap_or(Value::Null),
        answer: r.get(3)?,
        created_at: r.get(4)?,
    })
}

const INTEGRATION_COLS: &str =
    "id, run, target, expected, candidate, state, detail, close_task, owner, created_at, ended_at";

fn integration_row(r: &Row<'_>) -> rusqlite::Result<IntegrationRow> {
    let state: String = r.get(5)?;
    Ok(IntegrationRow {
        id: r.get(0)?,
        run: r.get(1)?,
        target: r.get(2)?,
        expected: r.get(3)?,
        candidate: r.get(4)?,
        state: IntegrationState::parse(&state),
        detail: r.get(6)?,
        close_task: r.get(7)?,
        owner: r.get(8)?,
        created_at: r.get(9)?,
        ended_at: r.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database written by schema v1 (before `usage`), with one run.
    fn v1_database(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("state.db");
        let v1 = SCHEMA.replace(
            ",\n    -- What the agent reported about tokens and cost (JSON), if anything.\n    usage            TEXT",
            "",
        );
        assert_ne!(v1, SCHEMA, "the v1 schema must not have the usage column");
        let c = Connection::open(&path).expect("open");
        c.execute_batch(&v1).expect("v1 schema");
        c.pragma_update(None, "user_version", 1).expect("version");
        c.execute(
            "INSERT INTO runs (id, task, agent, base, branch, worktree, state, created_at) VALUES ('old', 't', 'test', 'b', 'kitsu/run/old', '/tmp/wt', 'finished', 1)",
            [],
        )
        .expect("insert");
        path
    }

    #[test]
    fn v1_databases_upgrade_in_place_even_when_opened_concurrently() {
        let dir = std::env::temp_dir().join(format!("kitsu-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = v1_database(&dir);
        let opened: Vec<_> = (0..8)
            .map(|_| {
                let p = path.clone();
                std::thread::spawn(move || Store::open(&p).map(|_| ()))
            })
            .collect();
        for h in opened {
            h.join()
                .expect("thread")
                .expect("every opener sees a usable database");
        }
        let s = Store::open(&path).expect("reopen");
        let version: i64 = s
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("v");
        assert_eq!(version, SCHEMA_VERSION);
        let old = s.run("old").expect("old run survives");
        assert_eq!(old.usage, None);
        let u = Usage {
            input: Some(10),
            output: Some(2),
            ..Usage::default()
        };
        s.set_run_usage("old", &u).expect("usage");
        assert_eq!(s.run("old").expect("run").usage, Some(u));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn v2_databases_gain_judgments_and_keep_their_runs() {
        let dir = std::env::temp_dir().join(format!("kitsu-migrate-v2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("state.db");
        {
            // Schema v2 is SCHEMA as it stands, without JUDGMENTS.
            let c = Connection::open(&path).expect("open");
            c.execute_batch(SCHEMA).expect("v2 schema");
            c.pragma_update(None, "user_version", 2).expect("version");
            c.execute(
                "INSERT INTO runs (id, task, agent, base, branch, worktree, state, created_at) VALUES ('old', 't', 'test', 'b', 'kitsu/run/old', '/tmp/wt', 'finished', 1)",
                [],
            )
            .expect("insert");
            let has: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = 'judgments'",
                    [],
                    |r| r.get(0),
                )
                .expect("q");
            assert_eq!(has, 0, "v2 has no judgments table");
        }
        let s = Store::open(&path).expect("upgrade");
        let version: i64 = s
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("v");
        assert_eq!(version, 3);
        assert_eq!(
            s.run("old").expect("old run survives").state,
            RunState::Finished
        );
        let answers = serde_json::json!({ "low_risk": { "type": "yes_no", "p_yes": 0.97 } });
        let id = s
            .insert_judgment(&NewJudgment {
                run: Some("old"),
                purpose: "permission",
                kind: "yes_no",
                inputs: "abc",
                outcome: "answered",
                answers: &answers,
                reason: None,
                model: Some("jev-latest"),
                latency_ms: 12,
                attempts: 1,
                input_tokens: Some(200),
                output_tokens: Some(1),
                request_id: None,
            })
            .expect("insert");
        let row = s.judgment(id).expect("row");
        assert_eq!(row.answers, answers);
        assert_eq!(s.judgments_for_run("old").expect("list").len(), 1);
        let ev = s.run_events("old", 0, 10).expect("events");
        assert!(
            ev.iter()
                .any(|e| e.kind == "judge.done" && e.body["judgment"] == id)
        );
        drop(s);
        // Opening again changes nothing.
        let s = Store::open(&path).expect("reopen");
        assert_eq!(s.judgment(id).expect("still there").id, id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_folds_updates_and_turn_totals() {
        let mut u = Usage::default();
        u.apply_update(&serde_json::json!({ "used": 5000, "size": 200000, "cost": { "amount": 0.02, "currency": "USD" } }));
        u.apply_turn(&serde_json::json!({ "inputTokens": 1200, "outputTokens": 300, "cachedReadTokens": null, "totalTokens": 1500 }));
        // A later update without cost keeps the cost we already know.
        u.apply_update(&serde_json::json!({ "used": 6000, "size": 200000 }));
        assert_eq!(u.context_used, Some(6000));
        assert_eq!(u.cost, Some(0.02));
        assert_eq!(u.cached_read, None, "null means not reported");
        assert_eq!(u.spent(), Some(1500));
        assert_eq!(
            Usage {
                input: Some(7),
                ..Usage::default()
            }
            .spent(),
            Some(7)
        );
        assert_eq!(
            Usage {
                context_used: Some(9),
                ..Usage::default()
            }
            .spent(),
            None
        );
    }

    #[test]
    fn a_state_written_around_the_reducer_is_caught() {
        let s = store_with_run();
        s.apply_run_event("r1", &RunEvent::Started).expect("start");
        assert!(s.run_history_mismatches("r1").expect("check").is_empty());
        s.conn
            .execute("UPDATE runs SET state = 'finished' WHERE id = 'r1'", [])
            .expect("sneak");
        let bad = s.run_history_mismatches("r1").expect("check");
        assert_eq!(bad, ["stored state is finished, events end at running"]);
    }

    fn store_with_run() -> Store {
        let s = Store::open_in_memory().expect("store");
        s.insert_run(&NewRun {
            id: "r1",
            task: "t",
            agent: "test",
            base: "abc",
            branch: "kitsu/run/r1",
            worktree: "/tmp/wt",
            owner: "i1",
            from_run: None,
            note: None,
        })
        .expect("insert");
        s
    }

    #[test]
    fn transitions_are_transactional_and_logged() {
        let s = store_with_run();
        assert_eq!(
            s.apply_run_event("r1", &RunEvent::Started).expect("start"),
            Applied::Moved(RunState::Running)
        );
        assert_eq!(
            s.apply_run_event("r1", &RunEvent::TurnEnded("end_turn".into()))
                .expect("end"),
            Applied::Moved(RunState::Finished)
        );
        assert_eq!(
            s.apply_run_event("r1", &RunEvent::TurnEnded("end_turn".into()))
                .expect("dup"),
            Applied::Duplicate
        );
        let err = s
            .apply_run_event("r1", &RunEvent::CancelRequested)
            .expect_err("cancel after finish");
        assert_eq!(err.kind(), "invalid");
        let run = s.run("r1").expect("run");
        assert_eq!(run.state, RunState::Finished);
        assert_eq!(run.stop_reason.as_deref(), Some("end_turn"));
        assert!(run.ended_at.is_some());
        let kinds: Vec<String> = s
            .run_events("r1", 0, 100)
            .expect("events")
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(
            kinds,
            ["run.created", "run.state", "run.state", "run.duplicate"]
        );
    }

    #[test]
    fn resolution_happens_once() {
        let s = store_with_run();
        s.resolve_run("r1", "accepted").expect("first");
        assert_eq!(
            s.resolve_run("r1", "discarded").expect_err("second").kind(),
            "conflict"
        );
    }

    #[test]
    fn unbound_evidence_is_never_returned_for_a_tree() {
        let s = Store::open_in_memory().expect("store");
        let base = NewEvidence {
            check_name: "test",
            fingerprint: "fp",
            command: "true",
            tree: "t1",
            tree_after: Some("t2"),
            outcome: CheckOutcome::Pass,
            exit_code: Some(0),
            duration_ms: 5,
            log: None,
            log_bytes: None,
            run: None,
            started_at: 0,
        };
        s.insert_evidence(&base).expect("insert");
        assert!(s.evidence_at("fp", "t1").expect("q").is_none());
        assert!(s.evidence_at("fp", "t2").expect("q").is_none());
        s.insert_evidence(&NewEvidence {
            tree_after: Some("t1"),
            ..base
        })
        .expect("insert");
        assert_eq!(
            s.evidence_at("fp", "t1").expect("q").map(|e| e.outcome),
            Some(CheckOutcome::Pass)
        );
    }

    #[test]
    fn first_answer_wins() {
        let s = store_with_run();
        let id = s
            .insert_ask("r1", &serde_json::json!({ "title": "run npm install?" }))
            .expect("ask");
        assert!(s.answer_ask(id, "allow_once").expect("a1"));
        assert!(!s.answer_ask(id, "reject_once").expect("a2"));
        assert_eq!(
            s.ask(id).expect("ask").answer.as_deref(),
            Some("allow_once")
        );
    }

    #[test]
    fn set_meta_skips_identical_writes() {
        let s = Store::open_in_memory().expect("store");
        s.set_meta("k", "v").expect("set");
        let before = s.conn.total_changes();
        s.set_meta("k", "v").expect("set again");
        assert_eq!(
            s.conn.total_changes(),
            before,
            "identical value must not write"
        );
    }
}
