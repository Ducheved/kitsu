//! One contract, judged by diff.
//!
//! `.kitsu/kitsu.toml` says which checks guard which paths and which paths
//! are rules. This module answers the same three questions for every place
//! a change can be judged: an agent's stop hook (`kitsu gate stop`), a pull
//! request (`kitsu ci`) and a person reading a range (`kitsu diff`):
//!
//! - what changed, from which base, by content (tree hashes, so it doesn't
//!   matter which tool wrote a file or whether it was committed);
//! - which of those paths are rule changes, and the hash of exactly that
//!   diff that an approval has to name;
//! - which checks the change needs, and their evidence on this tree, running
//!   the ones that have none.
//!
//! The rules come from the base, never from the change: a change that edits
//! a check is judged by the check as it was, and the edit shows up as a rule
//! change.

use std::path::Path;

use serde::Serialize;

use crate::check::CheckRun;
use crate::error::{Error, Result};
use crate::git::Git;
use crate::intent::{self, CheckDef, Intent};
use crate::status::{RequiredCheck, guarded_checks, protected_changes};
use crate::store::{CheckOutcome, EvidenceRow, Store};
use crate::workspace::Workspace;

/// What a change is, relative to the base it is judged against.
#[derive(Debug, Clone)]
pub struct Change {
    /// A commit, or the empty tree when the repository has no commits.
    pub base: String,
    /// How the base was chosen, in words.
    pub base_why: String,
    pub from_tree: String,
    pub to_tree: String,
    pub changed: Vec<String>,
    /// The rules at `base`: the ones this change is judged by.
    pub rules: Intent,
}

/// The empty tree's id in this repository's hash format.
pub fn empty_tree(git: &Git) -> Result<String> {
    Ok(git.run(["mktree"])?.trim().to_string())
}

/// The rules as of `rev` (a commit or a tree).
pub fn rules_at(git: &Git, rev: &str) -> Result<Intent> {
    Ok(Intent::from_files(git.files_at(rev, intent::DIR)?))
}

/// The base an agent's work in `git`'s worktree is judged against, and why:
///
/// 1. `explicit` (`--base`), when given;
/// 2. in a linked worktree on another branch than your checkout (a Kitsu
///    run, `claude --worktree`): where it forked from your checkout's branch,
///    so commits the agent made there are part of the change;
/// 3. on a branch with an upstream: the merge base with it, so unpushed
///    commits are part of the change;
/// 4. otherwise `HEAD`: only uncommitted work. Commits made on a branch with
///    no upstream are not seen here; CI (`kitsu ci`) is where they are.
pub fn resolve_base(ws: &Workspace, git: &Git, explicit: Option<&str>) -> Result<(String, String)> {
    if let Some(b) = explicit {
        return Ok((git.rev(b)?, format!("--base {b}")));
    }
    let Some(head) = git.head()? else {
        return Ok((empty_tree(git)?, "no commits yet".into()));
    };
    let here = git.toplevel()?;
    let same = |a: &Path, b: &Path| {
        std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf())
            == std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf())
    };
    if !same(&here, &ws.root) {
        let main = ws.git();
        if let Some(branch) = main.current_branch()?
            && git.current_branch()?.as_deref() != Some(branch.as_str())
            && let Some(tip) = main.branch_head(&branch)?
        {
            return Ok((
                git.merge_base(&head, &tip)?,
                format!("where this worktree forked from {branch}"),
            ));
        }
    }
    if let Some(up) = git.upstream()? {
        return Ok((
            git.merge_base(&head, &up)?,
            "merge base with the upstream branch".into(),
        ));
    }
    Ok((head, "HEAD: uncommitted work only".into()))
}

impl Change {
    /// From `base` to the files on disk in `git`'s worktree.
    pub fn to_worktree(ws: &Workspace, git: &Git, base: (String, String)) -> Result<Change> {
        let to_tree = git.worktree_tree(&ws.scratch())?;
        Change::between(git, base, to_tree)
    }

    /// From `base` to a commit or tree.
    pub fn between(git: &Git, base: (String, String), to: String) -> Result<Change> {
        let (base, base_why) = base;
        let from_tree = git.tree_of(&base)?;
        let to_tree = git.tree_of(&to)?;
        Ok(Change {
            changed: git.changed_paths(&from_tree, &to_tree)?,
            rules: rules_at(git, &base)?,
            base,
            base_why,
            from_tree,
            to_tree,
        })
    }

    /// Changed paths under protected scopes, judged by the base's rules.
    pub fn rule_paths(&self) -> Vec<String> {
        protected_changes(&self.rules, &self.changed)
    }

    /// The hash an approval has to name: the same one `kitsu review` shows
    /// and `kitsu accept --approve` takes, over exactly these paths' diff.
    pub fn rule_token(&self, git: &Git) -> Result<Option<String>> {
        crate::integrate::protected_diff_token(
            git,
            &self.from_tree,
            &self.to_tree,
            &self.rule_paths(),
        )
    }
}

/// What kind of change a path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    Code,
    /// `.kitsu/**`: checks, tasks, decisions, questions, architecture.
    Rule,
    /// A memory note: protected, but a proposal of something learned.
    Memory,
    /// An agent hook config that decides whether the gate runs.
    HookConfig,
    /// A `[protect] paths` entry: tests, fixtures, CI config.
    Protected,
}

pub fn classify(rules: &Intent, path: &str) -> PathKind {
    if !rules.protected().contains(path) {
        PathKind::Code
    } else if intent::is_memory_note(path) {
        PathKind::Memory
    } else if path.starts_with(&format!("{}/", intent::DIR)) {
        PathKind::Rule
    } else if intent::HOOK_CONFIGS.contains(&path) {
        PathKind::HookConfig
    } else {
        PathKind::Protected
    }
}

/// A check whose definition differs between two sets of rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckChange {
    pub check: String,
    /// added, removed or changed
    pub change: &'static str,
    /// For `changed`: which fields.
    pub fields: Vec<&'static str>,
}

pub fn check_changes(before: &Intent, after: &Intent) -> Vec<CheckChange> {
    let (b, a) = (&before.config.checks, &after.config.checks);
    let mut out = Vec::new();
    for (name, old) in b {
        match a.get(name) {
            None => out.push(CheckChange {
                check: name.clone(),
                change: "removed",
                fields: Vec::new(),
            }),
            Some(new) => {
                let fields = changed_fields(old, new);
                if !fields.is_empty() {
                    out.push(CheckChange {
                        check: name.clone(),
                        change: "changed",
                        fields,
                    });
                }
            }
        }
    }
    for name in a.keys().filter(|n| !b.contains_key(*n)) {
        out.push(CheckChange {
            check: name.clone(),
            change: "added",
            fields: Vec::new(),
        });
    }
    out
}

fn changed_fields(a: &CheckDef, b: &CheckDef) -> Vec<&'static str> {
    let mut f = Vec::new();
    if a.run != b.run {
        f.push("run");
    }
    if a.timeout_secs != b.timeout_secs {
        f.push("timeout");
    }
    if a.scope != b.scope {
        f.push("scope");
    }
    if a.guards != b.guards {
        f.push("guards");
    }
    if a.why != b.why {
        f.push("why");
    }
    if a.held_out != b.held_out {
        f.push("held_out");
    }
    f
}

/// `kitsu diff`: every changed path with its kind, what the rule changes
/// are, and what the change needs.
#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    pub base: String,
    pub from_tree: String,
    pub to_tree: String,
    pub files: Vec<(String, PathKind)>,
    pub rule_paths: Vec<String>,
    /// The hash an approval of these rule changes must match.
    pub token: Option<String>,
    pub check_changes: Vec<CheckChange>,
    /// Checks the change needs, by the base's rules.
    pub required: Vec<RequiredCheck>,
}

pub fn diff_report(git: &Git, change: &Change, after: &Intent) -> Result<DiffReport> {
    Ok(DiffReport {
        base: change.base.clone(),
        from_tree: change.from_tree.clone(),
        to_tree: change.to_tree.clone(),
        files: change
            .changed
            .iter()
            .map(|p| (p.clone(), classify(&change.rules, p)))
            .collect(),
        rule_paths: change.rule_paths(),
        token: change.rule_token(git)?,
        check_changes: check_changes(&change.rules, after),
        required: guarded_checks(&change.rules, &change.changed),
    })
}

/// One required check on one tree: the row a receipt table prints.
#[derive(Debug, Clone, Serialize)]
pub struct Receipt {
    pub check: String,
    pub required_by: Vec<String>,
    pub fingerprint: String,
    pub tree: String,
    pub outcome: CheckOutcome,
    pub exit_code: Option<i32>,
    pub duration_ms: i64,
    pub evidence: i64,
    /// False when evidence for this exact tree already existed.
    pub ran: bool,
    /// The check changed files while it ran, so it describes neither tree.
    pub bound: bool,
    pub held_out: bool,
    #[serde(skip)]
    pub log: Option<String>,
}

impl Receipt {
    pub fn passed(&self) -> bool {
        self.outcome == CheckOutcome::Pass
    }
}

/// Evidence for every required check on `tree`, the files in `dir`: what is
/// recorded for exactly this tree is reused, anything else runs now.
pub fn verify(
    ws: &Workspace,
    store: &Store,
    dir: &Path,
    rules: &Intent,
    tree: &str,
    required: &[RequiredCheck],
) -> Result<Vec<Receipt>> {
    let cr = CheckRun {
        ws,
        store,
        dir,
        run: None,
    };
    let mut out = Vec::new();
    for req in required {
        let def = rules
            .config
            .checks
            .get(&req.name)
            .ok_or_else(|| Error::NotFound(format!("check {}", req.name)))?;
        let fp = def.fingerprint();
        let (row, ran): (EvidenceRow, bool) = match store.evidence_at(&fp, tree)? {
            Some(e) => (e, false),
            None => (cr.execute(def)?, true),
        };
        out.push(Receipt {
            check: req.name.clone(),
            required_by: req.why.clone(),
            fingerprint: fp,
            tree: row.tree.clone(),
            outcome: row.outcome,
            exit_code: row.exit_code,
            duration_ms: row.duration_ms,
            evidence: row.id,
            ran,
            bound: row.is_bound(),
            held_out: def.held_out,
            log: row.log.clone(),
        });
    }
    Ok(out)
}

/// The first and last part of a check's log, at most about `budget` bytes,
/// cut on character boundaries. The end of a test run usually says why it
/// failed; the start says what ran.
pub fn excerpt(text: &str, budget: usize) -> String {
    if text.len() <= budget {
        return text.to_string();
    }
    let head_len = budget / 3;
    let tail_len = budget - head_len;
    let mut h = head_len;
    while !text.is_char_boundary(h) {
        h -= 1;
    }
    let mut t = text.len() - tail_len;
    while !text.is_char_boundary(t) {
        t += 1;
    }
    format!(
        "{}\n[... {} bytes omitted ...]\n{}",
        &text[..h],
        t - h,
        &text[t..]
    )
}

pub fn log_text(ws: &Workspace, r: &Receipt) -> String {
    r.log
        .as_deref()
        .and_then(|id| ws.blobs().get(id).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// An approval is a record in this clone's state, never in the repository:
/// a change can't approve itself by carrying a file. It is not a lock
/// either: an agent with a shell running as you can run `kitsu gate
/// approve` too, which is why the hooks refuse that command from an agent.
pub fn is_approved(store: &Store, token: &str) -> Result<bool> {
    Ok(store.meta(&format!("approved.{token}"))?.is_some())
}

pub fn approve(store: &Store, token: &str) -> Result<()> {
    if token.len() < 16 || !token.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(Error::Invalid(format!(
            "`{token}` is not an approval hash (from `kitsu diff` or the gate's message)"
        )));
    }
    store.set_meta(
        &format!("approved.{token}"),
        &crate::util::now_ms().to_string(),
    )
}

// ---- kitsu ci ---------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CiOptions {
    pub base: Option<String>,
    /// The rule-change hash a person approved for this pull request.
    pub approved: Option<String>,
    /// Checks to run even if the diff doesn't touch what they guard.
    pub require: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CiReport {
    pub base: String,
    pub base_why: String,
    pub tree: String,
    pub changed: usize,
    pub rule_paths: Vec<String>,
    pub token: Option<String>,
    pub approved: bool,
    pub check_changes: Vec<CheckChange>,
    /// Rule files the change breaks (problems it adds).
    pub broken_rules: Vec<String>,
    pub receipts: Vec<Receipt>,
    pub failing: Vec<String>,
}

impl CiReport {
    pub fn unapproved_rule_change(&self) -> bool {
        self.token.is_some() && !self.approved
    }
}

/// The base of a pull request build: `--base`, else the first parent of the
/// checked-out merge commit (what `actions/checkout` gives a pull request),
/// else the merge base with `origin/$GITHUB_BASE_REF`.
pub fn ci_base(git: &Git, explicit: Option<&str>) -> Result<(String, String)> {
    if let Some(b) = explicit {
        return Ok((git.rev(b)?, format!("--base {b}")));
    }
    let parents = git.parents("HEAD")?;
    if parents.len() == 2 {
        return Ok((
            parents[0].clone(),
            "first parent of the merge commit".into(),
        ));
    }
    if let Some(r) = std::env::var("GITHUB_BASE_REF")
        .ok()
        .filter(|r| !r.is_empty())
    {
        let up = format!("origin/{r}");
        return Ok((
            git.merge_base("HEAD", &up)?,
            format!("merge base with {up}"),
        ));
    }
    Err(Error::Invalid(
        "no base to diff against: pass --base <rev> (a pull request's merge commit or GITHUB_BASE_REF is used when there is one)".into(),
    ))
}

pub fn ci(ws: &Workspace, store: &Store, dir: &Path, opts: &CiOptions) -> Result<CiReport> {
    let git = Git::new(dir);
    let base = ci_base(&git, opts.base.as_deref())?;
    let change = Change::to_worktree(ws, &git, base)?;
    if !change.rules.problems.is_empty() {
        return Err(Error::Denied(format!(
            "the base's rule files are broken, so the checks they define can't be enforced:\n  {}",
            problem_lines(&change.rules).join("\n  ")
        )));
    }
    let after = Intent::load_dir(dir)?;
    let before_problems = problem_lines(&change.rules);
    let broken_rules = problem_lines(&after)
        .into_iter()
        .filter(|p| !before_problems.contains(p))
        .collect();
    let mut required = guarded_checks(&change.rules, &change.changed);
    for name in &opts.require {
        if !change.rules.config.checks.contains_key(name) {
            return Err(Error::NotFound(format!(
                "check {name} (in the base's rules)"
            )));
        }
        if !required.iter().any(|r| &r.name == name) {
            required.push(RequiredCheck {
                name: name.clone(),
                why: vec!["--require".into()],
            });
        }
    }
    let receipts = verify(ws, store, dir, &change.rules, &change.to_tree, &required)?;
    let token = change.rule_token(&git)?;
    let approved = match (&token, &opts.approved) {
        (Some(t), Some(a)) => t == a.trim(),
        (None, _) => true,
        _ => false,
    };
    Ok(CiReport {
        failing: receipts
            .iter()
            .filter(|r| !r.passed())
            .map(|r| r.check.clone())
            .collect(),
        rule_paths: change.rule_paths(),
        check_changes: check_changes(&change.rules, &after),
        changed: change.changed.len(),
        base: change.base,
        base_why: change.base_why,
        tree: change.to_tree,
        token,
        approved,
        broken_rules,
        receipts,
    })
}

fn problem_lines(i: &Intent) -> Vec<String> {
    i.problems
        .iter()
        .map(|p| format!("{}: {}", p.path, p.detail))
        .collect()
}

pub fn short(id: &str) -> &str {
    &id[..id.len().min(10)]
}

/// The receipt table: one row per check, as Markdown so it reads the same in
/// a terminal and in a job summary.
pub fn receipt_table(receipts: &[Receipt]) -> String {
    let mut s = String::from(
        "| check | command | tree | exit | duration | result |\n|---|---|---|---|---|---|\n",
    );
    for r in receipts {
        let mut result = r.outcome.as_str().to_string();
        if !r.ran {
            result.push_str(" (recorded earlier)");
        }
        if !r.bound {
            result.push_str(" (changed files while running: not bound to the tree)");
        }
        if r.held_out {
            result.push_str(" (held out; an agent with a shell could have read it)");
        }
        s.push_str(&format!(
            "| {} | {} | {} | {} | {:.1}s | {} |\n",
            r.check,
            short(&r.fingerprint),
            short(&r.tree),
            r.exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into()),
            r.duration_ms as f64 / 1000.0,
            result
        ));
    }
    s
}

pub fn render_ci(r: &CiReport) -> String {
    let mut s = format!(
        "kitsu ci: {} changed paths from {} ({}), tree {}\n\n",
        r.changed,
        short(&r.base),
        r.base_why,
        short(&r.tree)
    );
    if r.receipts.is_empty() {
        s.push_str("No check guards what this change touches.\n");
    } else {
        s.push_str(&receipt_table(&r.receipts));
    }
    if !r.rule_paths.is_empty() {
        s.push_str(&format!(
            "\nRule changes ({}): {}\n",
            if r.approved {
                "approved"
            } else {
                "NOT approved"
            },
            r.rule_paths.join(", ")
        ));
        for c in &r.check_changes {
            s.push_str(&format!("  check `{}` {}", c.check, c.change));
            if !c.fields.is_empty() {
                s.push_str(&format!(": {}", c.fields.join(", ")));
            }
            s.push('\n');
        }
        if let Some(t) = &r.token {
            s.push_str(&format!("Approval hash for exactly this diff: {t}\n"));
        }
    }
    for p in &r.broken_rules {
        s.push_str(&format!("This change breaks a rule file: {p}\n"));
    }
    s
}

/// `key=value` lines for `$GITHUB_OUTPUT`.
pub fn github_outputs(r: &CiReport) -> String {
    format!(
        "rule-change={}\nrule-diff={}\napproved={}\nfailing={}\n",
        !r.rule_paths.is_empty(),
        r.token.as_deref().unwrap_or(""),
        r.approved,
        r.failing.join(",")
    )
}

pub fn render_diff(r: &DiffReport) -> String {
    let mut s = format!(
        "{}..{}: {} changed paths\n",
        short(&r.from_tree),
        short(&r.to_tree),
        r.files.len()
    );
    let code = r.files.iter().filter(|(_, k)| *k == PathKind::Code).count();
    s.push_str(&format!("  code: {code}\n"));
    for (p, k) in r.files.iter().filter(|(_, k)| *k != PathKind::Code) {
        let kind = match k {
            PathKind::Rule => "rule",
            PathKind::Memory => "memory note",
            PathKind::HookConfig => "hook config",
            PathKind::Protected => "protected",
            PathKind::Code => "code",
        };
        s.push_str(&format!("  {kind}: {p}\n"));
    }
    for c in &r.check_changes {
        s.push_str(&format!("  check `{}` {}", c.check, c.change));
        if !c.fields.is_empty() {
            s.push_str(&format!(": {}", c.fields.join(", ")));
        }
        s.push('\n');
    }
    if !r.required.is_empty() {
        s.push_str(&format!(
            "Needs: {}\n",
            r.required
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    match &r.token {
        Some(t) => s.push_str(&format!(
            "Rule changes need approval of exactly this diff: {t}\n"
        )),
        None => s.push_str("No rule changes.\n"),
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::TempRepo;

    const CFG: &str = "[checks.unit]\nrun = \"grep -q ok src/a.txt\"\nguards = [\"src/**\"]\n[checks.docs]\nrun = \"true\"\nguards = [\"docs/**\"]\n[protect]\npaths = [\"tests/**\"]\n";

    fn repo() -> TempRepo {
        TempRepo::new(&[
            (".kitsu/kitsu.toml", CFG),
            ("src/a.txt", "ok"),
            ("tests/t.sh", "true"),
            ("docs/x.md", "x"),
        ])
    }

    #[test]
    fn classifies_paths_and_check_edits() {
        let r = repo();
        let git = r.git();
        let before = rules_at(&git, "HEAD").expect("rules");
        assert_eq!(classify(&before, "src/a.txt"), PathKind::Code);
        assert_eq!(classify(&before, "tests/t.sh"), PathKind::Protected);
        assert_eq!(classify(&before, ".kitsu/kitsu.toml"), PathKind::Rule);
        assert_eq!(classify(&before, ".kitsu/memory/m.md"), PathKind::Memory);
        assert_eq!(
            classify(&before, ".claude/settings.json"),
            PathKind::HookConfig
        );

        let edited = CFG.replace("grep -q ok src/a.txt", "true").replace(
            "[checks.docs]\nrun = \"true\"\nguards = [\"docs/**\"]\n",
            "",
        ) + "[checks.new]\nrun = \"true\"\n";
        r.write(".kitsu/kitsu.toml", &edited);
        let after = Intent::load_dir(&r.root).expect("after");
        let got = check_changes(&before, &after);
        assert_eq!(
            got,
            vec![
                CheckChange {
                    check: "docs".into(),
                    change: "removed",
                    fields: vec![]
                },
                CheckChange {
                    check: "unit".into(),
                    change: "changed",
                    fields: vec!["run"]
                },
                CheckChange {
                    check: "new".into(),
                    change: "added",
                    fields: vec![]
                },
            ]
        );
    }

    #[test]
    fn a_change_is_judged_by_the_base_rules_and_its_token_is_accepts() {
        let r = repo();
        let git = r.git();
        let ws = Workspace::discover(&r.root).expect("ws");
        let base = git.head().expect("head").expect("commit");
        // The change weakens the check and edits code it guards.
        r.write(
            ".kitsu/kitsu.toml",
            &CFG.replace("grep -q ok src/a.txt", "true"),
        );
        r.write("src/a.txt", "bad");
        let change = Change::to_worktree(&ws, &git, (base.clone(), "test".into())).expect("change");
        assert_eq!(change.changed, [".kitsu/kitsu.toml", "src/a.txt"]);
        // Base rules: the original command, not the weakened one.
        assert_eq!(
            change.rules.config.checks["unit"].run,
            "grep -q ok src/a.txt"
        );
        assert_eq!(change.rule_paths(), [".kitsu/kitsu.toml"]);
        let token = change.rule_token(&git).expect("token").expect("some");
        let diff = git
            .diff(&change.from_tree, &change.to_tree, &[".kitsu/kitsu.toml"])
            .expect("diff");
        assert_eq!(token, crate::util::content_id(diff.as_bytes()));

        let store = ws.open_store().expect("store");
        let required = guarded_checks(&change.rules, &change.changed);
        let receipts = verify(
            &ws,
            &store,
            &r.root,
            &change.rules,
            &change.to_tree,
            &required,
        )
        .expect("verify");
        assert_eq!(receipts.len(), 1);
        assert!(receipts[0].ran && !receipts[0].passed());
        // Same tree again: the evidence is reused, nothing reruns.
        let again = verify(
            &ws,
            &store,
            &r.root,
            &change.rules,
            &change.to_tree,
            &required,
        )
        .expect("verify");
        assert!(!again[0].ran && again[0].evidence == receipts[0].evidence);

        assert!(!is_approved(&store, &token).expect("meta"));
        approve(&store, &token).expect("approve");
        assert!(is_approved(&store, &token).expect("meta"));
        assert!(approve(&store, "x; rm").is_err());
    }

    #[test]
    fn base_follows_where_the_work_is() {
        let r = repo();
        let git = r.git();
        let ws = Workspace::discover(&r.root).expect("ws");
        let head = git.head().expect("head").expect("commit");
        let (b, why) = resolve_base(&ws, &git, None).expect("base");
        assert_eq!(b, head);
        assert!(why.contains("uncommitted"), "{why}");

        // A linked worktree on its own branch, with a commit: judged from
        // where it forked, so the commit is part of the change.
        let wt = std::env::temp_dir().join(format!("kitsu-wt-{}", crate::util::short_id('w')));
        git.worktree_add(&wt, "agent", "HEAD").expect("worktree");
        std::fs::write(wt.join("src/a.txt"), "changed").expect("write");
        let g2 = Git::new(&wt);
        g2.run(["commit", "-qam", "agent work"]).expect("commit");
        let (b2, why2) = resolve_base(&ws, &g2, None).expect("base");
        assert_eq!(b2, head, "{why2}");
        let change = Change::to_worktree(&ws, &g2, (b2, why2)).expect("change");
        assert_eq!(change.changed, ["src/a.txt"]);
        git.worktree_remove(&wt).expect("remove");
    }

    #[test]
    fn excerpt_keeps_both_ends_on_char_boundaries() {
        let text = format!("{}{}{}", "é".repeat(100), "m".repeat(1000), "ü".repeat(100));
        let e = excerpt(&text, 300);
        assert!(e.starts_with('é') && e.ends_with('ü'), "{e}");
        assert!(e.contains("bytes omitted"));
        assert!(e.len() < 400);
        assert_eq!(excerpt("short", 300), "short");
    }
}
