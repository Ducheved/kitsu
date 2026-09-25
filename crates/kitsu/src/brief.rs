//! The brief: what an agent is told when it starts a task.
//!
//! Compiled, not remembered. Every run gets a fresh brief built from intent
//! at the run's base commit plus what the store knows about earlier
//! attempts. No model is involved and the output is deterministic for the
//! same inputs, which is what makes a task transferable between agents:
//! agent B gets the same constraints agent A got, plus A's recorded result.
//!
//! The brief says why each item is in it and lists what was left out for
//! space and how to get it. It never summarizes a source into something
//! stronger than the source: quoted rules link back to their files.

use std::fmt::Write as _;

use serde::Serialize;

use crate::check::status_at;
use crate::git::Git;
use crate::intent::{DecisionState, Intent, Memory, MemoryKind, MemoryState, QuestionState, Task};
use crate::memory::{self, Freshness, Personal};
use crate::run::RunState;
use crate::stats::estimate_tokens;
use crate::status::required_checks;
use crate::store::{RunRow, Store};

/// In estimated tokens (see `stats::estimate_tokens`), the unit the rest of
/// Kitsu reports in. Required parts are never trimmed, so a brief can go
/// over; only optional sections are dropped to fit.
pub const DEFAULT_BUDGET: usize = 6_000;

#[derive(Debug, Clone, Serialize)]
pub struct Included {
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    pub path: String,
    pub content_id: String,
    pub why: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Omitted {
    pub kind: &'static str,
    pub id: String,
    pub reason: String,
    pub reacquire: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Brief {
    pub task: String,
    pub markdown: String,
    pub included: Vec<Included>,
    pub omitted: Vec<Omitted>,
    /// Intent files that failed to parse. Always shown, never trimmed.
    pub problems: Vec<String>,
    /// The part that must survive the agent compacting its own context: the
    /// task, what done means, what must hold. Sent through a channel the
    /// agent re-sends on every request (Claude Code's system prompt) where
    /// one exists; the compaction probe showed the first user message alone
    /// doesn't survive (decision `rules-channel`).
    pub anchor: String,
}

pub struct Context<'a> {
    pub intent: &'a Intent,
    /// Needed to report check status at the base and the shape of earlier
    /// attempts. Optional so a brief can be printed outside a repo.
    pub git: Option<&'a Git>,
    pub store: Option<&'a Store>,
    /// Commit the agent starts from.
    pub base: Option<&'a str>,
    pub worktree: Option<&'a str>,
    pub run: Option<&'a RunRow>,
    /// The person's own notes (preferences), from their config dir.
    pub personal: &'a Personal,
    pub budget: usize,
}

struct Section {
    title: String,
    body: String,
    included: Vec<Included>,
    kind: &'static str,
    id: String,
    reacquire: String,
}

pub fn compile(cx: &Context<'_>, task: &Task) -> Brief {
    let intent = cx.intent;
    let mut out = String::new();
    let mut included = Vec::new();
    let mut omitted = Vec::new();

    let _ = writeln!(out, "# {}", task.title);
    let _ = writeln!(out);
    let _ = writeln!(out, "Task `{}` ({}).", task.id, task.source.path);
    included.push(Included {
        kind: "task",
        id: task.id.clone(),
        title: task.title.clone(),
        path: task.source.path.clone(),
        content_id: task.source.content_id.clone(),
        why: "the task".into(),
    });
    if !task.body.trim().is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", task.body.trim_end());
    }

    // ---- required: done means ----
    let anchor_from = out.len();
    let _ = writeln!(out, "\n## Done means");
    let base_tree = match (cx.git, cx.base) {
        (Some(g), Some(b)) => g.tree_of(b).ok(),
        _ => None,
    };
    let reqs = required_checks(intent, task, None);
    if reqs.is_empty() {
        let _ = writeln!(
            out,
            "- No checks are defined for this task. A human will judge the change by reading it."
        );
    }
    for r in &reqs {
        let def = &intent.config.checks[&r.name];
        let at_base = match (cx.git, cx.store, &base_tree) {
            (Some(g), Some(s), Some(t)) => status_at(g, s, def, t)
                .ok()
                .map(|st| format!(" On your base commit it is currently: {}.", st.word())),
            _ => None,
        };
        let _ = writeln!(
            out,
            "- Check `{}` passes: `{}` (required by {}).{}",
            r.name,
            def.shown_command(),
            r.why.join(", "),
            at_base.unwrap_or_default()
        );
        if let Some(why) = &def.why {
            for line in excerpt(why, 6).lines() {
                let _ = writeln!(out, "  {line}");
            }
        }
    }
    let _ = writeln!(
        out,
        "- A human reviews and accepts the change. You cannot mark the task done yourself; Kitsu runs the checks itself after you finish."
    );

    // ---- required: broken intent ----
    let problems: Vec<String> = intent
        .problems
        .iter()
        .map(|p| format!("{}: {}", p.path, p.detail))
        .chain(
            cx.personal
                .problems
                .iter()
                .map(|(p, d)| format!("{p}: {d}")),
        )
        .collect();
    if !problems.is_empty() {
        let _ = writeln!(out, "\n## Warning: some rule files could not be read");
        let _ = writeln!(
            out,
            "The constraints in these files are NOT in this brief. Do not assume they don't exist; ask if your change might touch them."
        );
        for p in &problems {
            let _ = writeln!(out, "- {p}");
        }
    }

    let anchor = format!(
        "# Kitsu rules for this run\n\nThis stays here when your context is compacted. The full brief is in the file named by $KITSU_BRIEF (or the `brief` tool of the `kitsu` MCP server); `orient` says where your checks stand now.\n\nTask `{}`: {}\n{}",
        task.id,
        task.title,
        &out[anchor_from..]
    );

    // ---- optional sections, in priority order ----
    let mut optional: Vec<Section> = Vec::new();

    for d in intent
        .decisions
        .values()
        .filter(|d| d.state != DecisionState::Superseded)
    {
        let by_scope = !d.scope.is_everything() && d.scope.may_overlap(&task.scope);
        let global = d.scope.is_everything();
        if !(by_scope || global) {
            continue;
        }
        let mut body = String::new();
        let settled = if d.state == DecisionState::Proposed {
            " (proposed, not settled)"
        } else {
            ""
        };
        let _ = writeln!(body, "- **{}**{settled} (`{}`)", d.title, d.id);
        for line in excerpt(&d.body, 10).lines() {
            let _ = writeln!(body, "  {line}");
        }
        for r in &d.rejected {
            let _ = writeln!(body, "  - Rejected: {r}");
        }
        let why = if by_scope {
            scope_reason(&task.scope, &d.scope)
        } else {
            "applies to the whole repository".into()
        };
        optional.push(Section {
            title: "Decisions that apply".into(),
            body,
            included: vec![Included {
                kind: "decision",
                id: d.id.clone(),
                title: d.title.clone(),
                path: d.source.path.clone(),
                content_id: d.source.content_id.clone(),
                why,
            }],
            kind: "decision",
            id: d.id.clone(),
            reacquire: format!("read {}", d.source.path),
        });
    }

    for q in intent
        .questions
        .values()
        .filter(|q| q.blocks.iter().any(|b| b == &task.id))
    {
        let mut body = String::new();
        match (&q.state, &q.answer) {
            (QuestionState::Answered, Some(a)) => {
                let _ = writeln!(body, "- {} (`{}`)\n  Answer: {}", q.title, q.id, a.trim());
            }
            _ => {
                let _ = writeln!(
                    body,
                    "- {} (`{}`) is still OPEN. Don't guess the answer; if your change depends on it, stop and say so.",
                    q.title, q.id
                );
            }
        }
        optional.push(Section {
            title: "Questions about this task".into(),
            body,
            included: vec![Included {
                kind: "question",
                id: q.id.clone(),
                title: q.title.clone(),
                path: q.source.path.clone(),
                content_id: q.source.content_id.clone(),
                why: "blocks this task".into(),
            }],
            kind: "question",
            id: q.id.clone(),
            reacquire: format!("read {}", q.source.path),
        });
    }

    // Notes: gotchas and conventions first, stale ones after current ones.
    let fresh = match (cx.git, cx.base) {
        (Some(g), Some(b)) => memory::freshness(g, b, intent.memory.values()).ok(),
        _ => None,
    };
    // Retired and superseded notes stay out, and say so: "we used to
    // believe X" is worth a line, not a place among current notes.
    let superseded = intent.superseded();
    let mut notes: Vec<(&Memory, Option<&Freshness>, String)> = intent
        .memory
        .values()
        .filter(|m| {
            let gone = if m.state == MemoryState::Retired {
                Some("retired".to_string())
            } else {
                superseded
                    .get(&m.id)
                    .map(|by| format!("superseded by `{by}`"))
            };
            if let Some(reason) = gone {
                if m.scope.is_everything() || m.scope.may_overlap(&task.scope) {
                    omitted.push(Omitted {
                        kind: "memory",
                        id: m.id.clone(),
                        reason,
                        reacquire: format!("read {}", m.source.path),
                    });
                }
                return false;
            }
            true
        })
        .filter_map(|m| {
            let why = if m.scope.is_everything() {
                "applies to the whole repository".to_string()
            } else if m.scope.may_overlap(&task.scope) {
                scope_reason(&task.scope, &m.scope)
            } else {
                return None;
            };
            Some((m, fresh.as_ref().and_then(|f| f.get(&m.id)), why))
        })
        .chain(
            cx.personal
                .notes
                .iter()
                .map(|m| (m, None, "your human's preference".to_string())),
        )
        .collect();
    let rank = |k: MemoryKind| match k {
        MemoryKind::Gotcha => 0,
        MemoryKind::Convention => 1,
        MemoryKind::Preference => 2,
        MemoryKind::Fact => 3,
        MemoryKind::Lesson => 4,
    };
    notes.sort_by_key(|(m, f, _)| {
        (
            matches!(f, Some(Freshness::Stale { .. })),
            rank(m.kind),
            m.id.clone(),
        )
    });
    let mut first_note = true;
    for (m, f, why) in notes {
        let mut body = String::new();
        if std::mem::take(&mut first_note) {
            let _ = writeln!(
                body,
                "Notes are what earlier work believed, with where it came from. They can be wrong or out of date; if one disagrees with a rule above or with the code, the rule or the code wins."
            );
        }
        let _ = writeln!(body, "- **{}** ({}, `{}`)", m.title, m.kind.as_str(), m.id);
        if let Some(Freshness::Stale { changed, since }) = f {
            let _ = writeln!(
                body,
                "  May be out of date: {} changed after it was written ({}). Check before relying on it.",
                changed.join(", "),
                short(since)
            );
        }
        for line in excerpt(&m.body, 8).lines() {
            let _ = writeln!(body, "  {line}");
        }
        let reacquire = format!("read {}", m.source.path);
        optional.push(Section {
            title: "What earlier work learned".into(),
            body,
            included: vec![Included {
                kind: "memory",
                id: m.id.clone(),
                title: m.title.clone(),
                path: m.source.path.clone(),
                content_id: m.source.content_id.clone(),
                why,
            }],
            kind: "memory",
            id: m.id.clone(),
            reacquire,
        });
    }

    if let (Some(store), Some(git)) = (cx.store, cx.git) {
        let current = cx.run.map(|r| r.id.as_str());
        let attempts: Vec<RunRow> = store
            .runs_for_task(&task.id)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| Some(r.id.as_str()) != current)
            .take(3)
            .collect();
        for r in attempts {
            let body = describe_attempt(git, store, &r);
            optional.push(Section {
                title: "Earlier attempts".into(),
                body,
                included: vec![],
                kind: "run",
                id: r.id.clone(),
                reacquire: format!("kitsu show {}", r.id),
            });
        }
    }

    let mut last_title = String::new();
    for s in optional {
        let needed = s.body.len()
            + if s.title != last_title {
                s.title.len() + 5
            } else {
                0
            };
        if estimate_tokens((out.len() + needed) as u64) > cx.budget as u64 {
            omitted.push(Omitted {
                kind: s.kind,
                id: s.id,
                reason: "over the brief budget".into(),
                reacquire: s.reacquire,
            });
            continue;
        }
        if s.title != last_title {
            let _ = writeln!(out, "\n## {}", s.title);
            last_title = s.title.clone();
        }
        out.push_str(&s.body);
        included.extend(s.included);
    }

    // ---- continuation and note ----
    if let Some(run) = cx.run {
        if let Some(from) = &run.from_run {
            let _ = writeln!(out, "\n## You are continuing earlier work");
            let _ = writeln!(
                out,
                "Your worktree already contains the changes from run `{from}`. Build on them; don't start over unless they are wrong."
            );
        }
        if let Some(note) = run.note.as_deref().filter(|n| !n.trim().is_empty()) {
            let _ = writeln!(out, "\n## Note from the human who started this run");
            let _ = writeln!(out, "{}", note.trim());
        }
    }

    // ---- required: working agreement ----
    let _ = writeln!(out, "\n## How to work here");
    if let (Some(wt), Some(base)) = (cx.worktree, cx.base) {
        let _ = writeln!(
            out,
            "- You are in an isolated git worktree at `{wt}`, based on `{}`. Only edit files inside it.",
            short(base)
        );
    }
    let _ = writeln!(
        out,
        "- If you have Kitsu's MCP tools (server `kitsu`), `orient` tells you where your checks stand on your current files, `search` finds code without reading files, `rules_for` lists what covers a path. They answer from recorded state and cost less than exploring."
    );
    let _ = writeln!(
        out,
        "- If you need a decision from a human, write `.kitsu/questions/<short-name>.md` (front matter: `title`, `blocks = [\"{}\"]`) and stop.",
        task.id
    );
    let _ = writeln!(
        out,
        "- If you make a design decision, write or update `.kitsu/decisions/<short-name>.md` with `state = \"proposed\"`. It is reviewed separately from your code."
    );
    let _ = writeln!(
        out,
        "- If you learn something the next agent would otherwise rediscover, write `.kitsu/memory/<short-name>.md` (front matter: `title`, `kind` = fact, gotcha, convention or lesson, `scope`, and `anchors` = the files it describes, so it's flagged when they change)."
    );
    let _ = writeln!(
        out,
        "- Changes under `.kitsu/` and other protected paths are shown to the reviewer as rule changes. Weakening a check to get green will be seen."
    );
    let _ = writeln!(
        out,
        "- You can commit or leave changes uncommitted; Kitsu snapshots the worktree when you finish."
    );

    if !omitted.is_empty() {
        let _ = writeln!(out, "\n## Left out of this brief");
        for o in &omitted {
            let _ = writeln!(
                out,
                "- {} `{}` ({}); to read it: `{}`",
                o.kind, o.id, o.reason, o.reacquire
            );
        }
    }

    Brief {
        task: task.id.clone(),
        markdown: out,
        included,
        omitted,
        problems,
        anchor,
    }
}

fn describe_attempt(git: &Git, store: &Store, r: &RunRow) -> String {
    let mut s = String::new();
    let outcome = match r.state {
        RunState::Finished => format!("finished ({})", r.stop_reason.as_deref().unwrap_or("?")),
        other => other.as_str().to_string(),
    };
    let resolution = r
        .resolution
        .as_deref()
        .map(|x| format!(", {x}"))
        .unwrap_or_default();
    let _ = writeln!(s, "- Run `{}` by {}: {outcome}{resolution}.", r.id, r.agent);
    if let Some(d) = &r.detail {
        let _ = writeln!(s, "  Detail: {d}");
    }
    if let Some(snap) = &r.snapshot
        && let Ok(stats) = git.numstat(&r.base, snap)
        && !stats.is_empty()
    {
        let files: Vec<String> = stats
            .iter()
            .take(8)
            .map(|f| match (f.added, f.removed) {
                (Some(a), Some(d)) => format!("{} (+{a} -{d})", f.path),
                _ => format!("{} (binary)", f.path),
            })
            .collect();
        let more = stats.len().saturating_sub(8);
        let more = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        let _ = writeln!(s, "  Changed: {}{more}.", files.join(", "));
    }
    // Evidence tagged with the run is not all about the run's own change:
    // accept checks the change combined with the target branch, and a
    // check that edits files proves nothing about the tree it started on.
    let evidence = store.evidence_for_run(&r.id).unwrap_or_default();
    let (own, other): (Vec<_>, Vec<_>) = evidence.iter().partition(|e| {
        r.snapshot_tree.as_deref() == Some(e.tree.as_str()) && e.tree_after.is_none()
    });
    let list = |v: &[&crate::store::EvidenceRow]| {
        v.iter()
            .map(|e| format!("{} {}", e.check_name, e.outcome.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !own.is_empty() {
        let _ = writeln!(s, "  Checks on its own change: {}.", list(&own));
    }
    if !other.is_empty() {
        let _ = writeln!(
            s,
            "  Checks on other trees (combined with the target branch at accept, or a check that edited files): {}.",
            list(&other)
        );
    }
    if let Some(n) = r.note.as_deref().filter(|n| !n.trim().is_empty()) {
        let _ = writeln!(s, "  Note given to it: {}", n.trim());
    }
    s
}

fn scope_reason(task: &crate::scope::Scope, other: &crate::scope::Scope) -> String {
    if other.is_everything() {
        "applies to the whole repository".into()
    } else if task.is_everything() {
        format!(
            "task has no declared scope; this covers {}",
            other.globs().join(", ")
        )
    } else {
        format!(
            "task scope {} overlaps {}",
            task.globs().join(", "),
            other.globs().join(", ")
        )
    }
}

fn excerpt(body: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = body.trim().lines().collect();
    if lines.len() <= max_lines {
        return lines.join("\n");
    }
    let mut s = lines[..max_lines].join("\n");
    let _ = write!(s, "\n({} more lines in the file)", lines.len() - max_lines);
    s
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(10)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(files: &[(&str, &str)]) -> Intent {
        Intent::from_files(
            files
                .iter()
                .map(|(p, c)| (p.to_string(), c.as_bytes().to_vec()))
                .collect(),
        )
    }

    fn fixture() -> Intent {
        intent(&[
            (
                ".kitsu/kitsu.toml",
                "[checks.unit]\nrun = \"cargo test\"\n[checks.idem]\nrun = \"cargo test idem\"\nguards = [\"src/client/**\"]\nwhy = \"Stable idempotency key: it comes from the operation, never the attempt.\"\n[checks.ui]\nrun = \"npm test\"\nguards = [\"app/**\"]\nwhy = \"UI never blocks\"\n",
            ),
            (
                ".kitsu/tasks/retry.md",
                "+++\ntitle = \"Bound retries\"\nscope = [\"src/client/**\"]\nchecks = [\"unit\"]\n+++\nUpstream times out.\n",
            ),
            (
                ".kitsu/decisions/bounded.md",
                "+++\ntitle = \"At most 3 attempts\"\nrejected = [\"infinite retry: amplifies outages\"]\nscope = [\"src/client/**\"]\n+++\n",
            ),
            (
                ".kitsu/questions/dedupe.md",
                "+++\ntitle = \"Does upstream dedupe keys?\"\nblocks = [\"retry\"]\nstate = \"answered\"\nanswer = \"Yes, for 24h.\"\n+++\n",
            ),
        ])
    }

    fn cx(intent: &Intent, budget: usize) -> Context<'_> {
        Context {
            intent,
            git: None,
            store: None,
            base: None,
            worktree: None,
            run: None,
            personal: &NO_PERSONAL,
            budget,
        }
    }

    static NO_PERSONAL: Personal = Personal {
        notes: Vec::new(),
        problems: Vec::new(),
    };

    #[test]
    fn includes_what_constrains_the_task_and_says_why() {
        let i = fixture();
        let b = compile(&cx(&i, DEFAULT_BUDGET), &i.tasks["retry"]);
        let md = &b.markdown;
        assert!(md.contains("Stable idempotency key"));
        assert!(md.contains("Rejected: infinite retry"));
        assert!(md.contains("Answer: Yes, for 24h."));
        assert!(
            md.contains("`idem` passes") && md.contains("(required by guards src/client/**)"),
            "a guarding check becomes required:\n{md}"
        );
        assert!(
            !md.contains("UI never blocks") && !md.contains("`ui` passes"),
            "a check guarding other paths leaked in"
        );
        let d = b
            .included
            .iter()
            .find(|x| x.id == "bounded")
            .expect("decision in scope included");
        assert!(d.why.contains("overlaps"));
        // What a check protects rides in the part that survives compaction.
        assert!(
            b.anchor.contains("comes from the operation"),
            "{}",
            b.anchor
        );
    }

    #[test]
    fn is_deterministic() {
        let i = fixture();
        let a = compile(&cx(&i, DEFAULT_BUDGET), &i.tasks["retry"]).markdown;
        let b = compile(&cx(&i, DEFAULT_BUDGET), &i.tasks["retry"]).markdown;
        assert_eq!(a, b);
    }

    #[test]
    fn budget_drops_optional_sections_but_never_constraints() {
        let i = fixture();
        let b = compile(&cx(&i, 25), &i.tasks["retry"]);
        assert!(
            b.markdown.contains("Stable idempotency key"),
            "what a required check protects is never trimmed"
        );
        assert!(
            b.omitted.iter().any(|o| o.id == "bounded"),
            "{:?}",
            b.omitted
        );
        assert!(b.markdown.contains("Left out of this brief"));
    }

    #[test]
    fn memory_in_scope_is_included_gotchas_first_and_personal_notes_ride_along() {
        let mut files = vec![
            (
                ".kitsu/tasks/t.md",
                "+++\nscope = [\"src/client/**\"]\n+++\n",
            ),
            (
                ".kitsu/memory/fact.md",
                "+++\ntitle = \"Upstream dedupes for 24h\"\nkind = \"fact\"\nscope = [\"src/client/**\"]\n+++\n",
            ),
            (
                ".kitsu/memory/trap.md",
                "+++\ntitle = \"Tests pass offline only because of a stub\"\nkind = \"gotcha\"\nscope = [\"src/**\"]\n+++\n",
            ),
            (
                ".kitsu/memory/ui.md",
                "+++\ntitle = \"The UI polls\"\nkind = \"fact\"\nscope = [\"app/**\"]\n+++\n",
            ),
        ];
        files.sort();
        let i = intent(&files);
        let personal = Personal {
            notes: vec![
                crate::intent::parse_memory(
                    "personal/small",
                    "kind = \"preference\"\ntitle = \"Small commits\"",
                    "",
                    crate::intent::Source {
                        path: "~/.config/kitsu/memory/small.md".into(),
                        content_id: "x".into(),
                    },
                )
                .expect("note"),
            ],
            problems: vec![("~/.config/kitsu/memory/bad.md".into(), "nope".into())],
        };
        let mut c = cx(&i, DEFAULT_BUDGET);
        c.personal = &personal;
        let b = compile(&c, &i.tasks["t"]);
        let md = &b.markdown;
        let trap = md.find("Tests pass offline").expect("gotcha included");
        let fact = md.find("Upstream dedupes").expect("fact included");
        assert!(trap < fact, "gotchas before facts:\n{md}");
        assert!(!md.contains("The UI polls"), "out-of-scope note leaked in");
        assert!(md.contains("Small commits"));
        assert!(
            md.contains("bad.md: nope"),
            "a broken personal note is reported, not dropped"
        );
        let why = &b
            .included
            .iter()
            .find(|x| x.id == "personal/small")
            .expect("personal")
            .why;
        assert_eq!(why, "your human's preference");
    }

    #[test]
    fn stale_notes_say_what_changed() {
        use crate::git::testing::TempRepo;
        let repo = TempRepo::new(&[
            ("src/client.rs", "fn call() {}\n"),
            (".kitsu/tasks/t.md", "+++\nscope = [\"src/**\"]\n+++\n"),
            (
                ".kitsu/memory/call.md",
                "+++\ntitle = \"call() never retries\"\nkind = \"fact\"\nscope = [\"src/**\"]\nanchors = [\"src/client.rs\"]\n+++\n",
            ),
        ]);
        repo.write("src/client.rs", "fn call() { retry() }\n");
        let head = repo.commit_all("retry");
        let git = repo.git();
        let i = Intent::from_files(git.files_at(&head, crate::intent::DIR).expect("files"));
        let mut c = cx(&i, DEFAULT_BUDGET);
        c.git = Some(&git);
        c.base = Some(&head);
        let md = compile(&c, &i.tasks["t"]).markdown;
        assert!(
            md.contains("May be out of date: src/client.rs changed after it was written"),
            "{md}"
        );
    }

    #[test]
    fn broken_rule_files_are_loud() {
        let i = intent(&[
            (".kitsu/tasks/t.md", "+++\n+++\n"),
            (".kitsu/decisions/bad.md", "+++\nnope\n+++\n"),
        ]);
        let b = compile(&cx(&i, DEFAULT_BUDGET), &i.tasks["t"]);
        assert!(b.markdown.contains("could not be read"));
        assert_eq!(b.problems.len(), 1);
    }
}
