//! Intent: the part of engineering state that people review.
//!
//! Tasks, decisions, open questions, memory notes and the architecture
//! model live as Markdown files
//! with TOML front matter under `.kitsu/` in the repository. That makes git
//! their authority: they branch, merge, conflict and get reviewed with the
//! code they talk about, and they stay readable if Kitsu goes away.
//!
//! Nothing in here is execution state. Whether a task is running, verified
//! or blocked is derived elsewhere from runs and evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::scope::Scope;
use crate::util::content_id;

pub const DIR: &str = ".kitsu";
pub const CONFIG: &str = ".kitsu/kitsu.toml";

/// The agents' hook configs `kitsu hooks install` writes. They decide
/// whether the gate runs at all, so changing one is a rule change, like
/// changing `.kitsu/`.
pub const HOOK_CONFIGS: &[&str] = &[
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".codex/hooks.json",
    ".codex/config.toml",
    ".cursor/hooks.json",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Task,
    Decision,
    Question,
    Memory,
    Architecture,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Task,
        Kind::Decision,
        Kind::Question,
        Kind::Memory,
        Kind::Architecture,
    ];

    pub fn dir(self) -> &'static str {
        match self {
            Kind::Task => "tasks",
            Kind::Decision => "decisions",
            Kind::Question => "questions",
            Kind::Memory => "memory",
            Kind::Architecture => "architecture",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Task => "task",
            Kind::Decision => "decision",
            Kind::Question => "question",
            Kind::Memory => "memory",
            Kind::Architecture => "element",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL
            .into_iter()
            .find(|k| k.name() == s || k.dir() == s)
    }

    fn from_dir(dir: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.dir() == dir)
    }
}

/// Where an entity came from, precisely enough to tell whether a brief that
/// quoted it is still current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub path: String,
    pub content_id: String,
}

#[derive(Debug, Clone)]
pub struct CheckDef {
    pub name: String,
    pub run: String,
    pub timeout_secs: u64,
    /// Evidence stays fresh while changes stay outside this scope.
    /// Empty means any change makes it stale.
    pub scope: Scope,
    /// A change that touches these paths needs this check to pass, whatever
    /// its task says. Empty: required only by tasks that name it.
    pub guards: Scope,
    /// What the check protects, in words. Goes into the brief of every task
    /// it guards.
    pub why: Option<String>,
    /// The check's files live outside the repository (see
    /// `check::held_out_dir`), and its command and output are never shown
    /// to an agent. Out of sight, not secret: an agent with a shell can
    /// still go and read them.
    pub held_out: bool,
}

impl CheckDef {
    /// The command as an agent may see it. A held-out check's command can
    /// name the files it keeps out of sight.
    pub fn shown_command(&self) -> &str {
        if self.held_out {
            "(held out: its tests are not in this repository)"
        } else {
            &self.run
        }
    }

    /// Identity of what the check *does*. Editing the command invalidates old
    /// evidence; editing the freshness scope does not change past results.
    pub fn fingerprint(&self) -> String {
        content_id(format!("{}\0{}", self.run, self.timeout_secs).as_bytes())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub checks: BTreeMap<String, CheckDef>,
    /// Changes under these paths need explicit approval when a run is
    /// accepted. `.kitsu/**` is always included.
    pub protect: Scope,
    /// Files that must each belong to exactly one architecture component
    /// (`[architecture] cover`). Empty: coverage isn't checked.
    pub architecture_cover: Scope,
}

/// C4 level of an architecture element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Person,
    External,
    System,
    Container,
    Component,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Person => "person",
            Level::External => "external",
            Level::System => "system",
            Level::Container => "container",
            Level::Component => "component",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementState {
    Active,
    /// Described before it exists: its paths may not match anything yet.
    Planned,
    Retired,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Uses {
    pub to: String,
    pub why: Option<String>,
}

/// One element of the architecture model (`.kitsu/architecture/<id>.md`).
/// The body is its documentation; level 4 (code) is never stored.
#[derive(Debug, Clone)]
pub struct Element {
    pub id: String,
    pub title: String,
    pub level: Level,
    pub parent: Option<String>,
    pub technology: Option<String>,
    /// The files this element claims (globs).
    pub paths: Vec<String>,
    pub uses: Vec<Uses>,
    /// Prose outside `.kitsu/` that describes this element.
    pub docs: Vec<String>,
    pub state: ElementState,
    pub body: String,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Open,
    Done,
    Dropped,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub state: TaskState,
    pub scope: Scope,
    pub checks: Vec<String>,
    pub after: Vec<String>,
    pub body: String,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionState {
    Proposed,
    Accepted,
    Superseded,
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub id: String,
    pub title: String,
    pub state: DecisionState,
    pub scope: Scope,
    pub rejected: Vec<String>,
    pub supersedes: Vec<String>,
    pub body: String,
    pub source: Source,
}

/// What sort of thing a memory note records. The kind decides how it's
/// presented, not whether it's true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Something true about the system: "upstream dedupes keys for 24h".
    Fact,
    /// A trap: "the test suite passes with the network off only because...".
    Gotcha,
    /// How things are done here: "errors are wrapped with context, never logged and returned".
    Convention,
    /// What a person wants: "small commits", "no new dependencies without asking".
    Preference,
    /// What an earlier attempt taught: "mocking the clock hid the retry bug".
    Lesson,
}

impl MemoryKind {
    pub const ALL: [MemoryKind; 5] = [
        MemoryKind::Fact,
        MemoryKind::Gotcha,
        MemoryKind::Convention,
        MemoryKind::Preference,
        MemoryKind::Lesson,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Fact => "fact",
            MemoryKind::Gotcha => "gotcha",
            MemoryKind::Convention => "convention",
            MemoryKind::Preference => "preference",
            MemoryKind::Lesson => "lesson",
        }
    }

    pub fn parse(s: &str) -> Option<MemoryKind> {
        MemoryKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// A note worth keeping between runs, with where it came from and what it
/// depends on. `anchors` are the paths whose content the note describes;
/// when they change after the note was last committed, the note is stale
/// (see `memory::freshness`). No hashes in the file: git knows when the
/// note was written and what the anchors looked like then.
#[derive(Debug, Clone)]
pub struct Memory {
    pub id: String,
    pub title: String,
    pub kind: MemoryKind,
    /// Where it applies, for deciding which briefs include it.
    pub scope: Scope,
    pub anchors: Scope,
    /// Who wrote it: a person or an agent name.
    pub by: Option<String>,
    /// The run it came out of, if any.
    pub run: Option<String>,
    /// `retired`: someone found it no longer holds. Kept, not deleted, so
    /// the next agent can see what used to be believed and why it stopped.
    pub state: MemoryState,
    /// Notes this one replaces. "Superseded" is derived from this on read
    /// (`Intent::superseded`); the old note is never edited.
    pub supersedes: Vec<String>,
    /// Declares a single-valued fact ("payments.retry_budget"): two live
    /// notes with the same key are a problem, never settled by recency.
    pub key: Option<String>,
    /// Why it was retired or what changed, one line.
    pub reason: Option<String>,
    pub body: String,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryState {
    Current,
    Retired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionState {
    Open,
    Answered,
}

#[derive(Debug, Clone)]
pub struct Question {
    pub id: String,
    pub title: String,
    pub state: QuestionState,
    pub blocks: Vec<String>,
    pub answer: Option<String>,
    pub body: String,
    pub source: Source,
}

/// A file Kitsu could not use. Problems are never dropped on the floor: a
/// broken rule file means a constraint is missing, and the brief and the
/// status view both say so.
#[derive(Debug, Clone)]
pub struct Problem {
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct Intent {
    pub config: Config,
    pub tasks: BTreeMap<String, Task>,
    pub decisions: BTreeMap<String, Decision>,
    pub questions: BTreeMap<String, Question>,
    pub memory: BTreeMap<String, Memory>,
    pub architecture: BTreeMap<String, Element>,
    pub problems: Vec<Problem>,
}

impl Intent {
    /// Read `.kitsu/` from a working directory.
    pub fn load_dir(root: &Path) -> Result<Intent> {
        Ok(Intent::from_files(dir_files(root)?))
    }

    /// Build from `(repo-relative path, contents)` pairs. Used for both a
    /// working directory and a git commit, so the two can never parse
    /// differently.
    pub fn from_files(files: Vec<(String, Vec<u8>)>) -> Intent {
        let mut intent = Intent::default();
        for (path, bytes) in files {
            let source = Source {
                content_id: content_id(&bytes),
                path: path.clone(),
            };
            let text = match String::from_utf8(bytes) {
                Ok(t) => t,
                Err(_) => {
                    intent.problem(&path, "not valid UTF-8");
                    continue;
                }
            };
            if path == CONFIG {
                match parse_config(&text) {
                    Ok(c) => intent.config = c,
                    Err(e) => intent.problem(&path, e),
                }
                continue;
            }
            if is_retired_invariant(&path) {
                intent.problem(&path, RETIRED_INVARIANTS);
                continue;
            }
            let Some((kind, id)) = classify(&path) else {
                continue;
            };
            if let Err(e) = valid_id(id) {
                intent.problem(&path, e);
                continue;
            }
            if let Err(e) = intent.add(kind, id, &text, source) {
                intent.problem(&path, e);
            }
        }
        intent.validate();
        intent
    }

    fn problem(&mut self, path: &str, detail: impl Into<String>) {
        self.problems.push(Problem {
            path: path.to_string(),
            detail: detail.into(),
        });
    }

    fn add(&mut self, kind: Kind, id: &str, text: &str, source: Source) -> Result<(), String> {
        let (front, body) = split_front_matter(text)?;
        let title_fallback = || first_heading(body).unwrap_or_else(|| id.to_string());
        let id = id.to_string();
        match kind {
            Kind::Task => {
                let f: TaskFront = toml::from_str(front).map_err(|e| toml_msg(&e))?;
                let state = match f.state.as_deref().unwrap_or("open") {
                    "open" => TaskState::Open,
                    "done" => TaskState::Done,
                    "dropped" => TaskState::Dropped,
                    other => {
                        return Err(format!(
                            "unknown task state `{other}` (open, done, dropped)"
                        ));
                    }
                };
                let title = f.title.unwrap_or_else(title_fallback);
                self.tasks.insert(
                    id.clone(),
                    Task {
                        id,
                        title,
                        state,
                        scope: Scope::new(f.scope),
                        checks: f.checks,
                        after: f.after,
                        body: body.to_string(),
                        source,
                    },
                );
            }
            Kind::Decision => {
                let f: DecisionFront = toml::from_str(front).map_err(|e| toml_msg(&e))?;
                let state = match f.state.as_deref().unwrap_or("accepted") {
                    "proposed" => DecisionState::Proposed,
                    "accepted" => DecisionState::Accepted,
                    "superseded" => DecisionState::Superseded,
                    other => {
                        return Err(format!(
                            "unknown decision state `{other}` (proposed, accepted, superseded)"
                        ));
                    }
                };
                let title = f.title.unwrap_or_else(title_fallback);
                self.decisions.insert(
                    id.clone(),
                    Decision {
                        id,
                        title,
                        state,
                        scope: Scope::new(f.scope),
                        rejected: f.rejected,
                        supersedes: f.supersedes,
                        body: body.to_string(),
                        source,
                    },
                );
            }
            Kind::Memory => {
                let m = parse_memory(&id, front, body, source)?;
                self.memory.insert(id, m);
            }
            Kind::Architecture => {
                let f: ElementFront = toml::from_str(front).map_err(|e| toml_msg(&e))?;
                let level = match f.level.as_str() {
                    "person" => Level::Person,
                    "external" => Level::External,
                    "system" => Level::System,
                    "container" => Level::Container,
                    "component" => Level::Component,
                    other => {
                        return Err(format!(
                            "unknown level `{other}` (person, external, system, container, component)"
                        ));
                    }
                };
                let state = match f.state.as_deref().unwrap_or("active") {
                    "active" => ElementState::Active,
                    "planned" => ElementState::Planned,
                    "retired" => ElementState::Retired,
                    other => {
                        return Err(format!(
                            "unknown element state `{other}` (active, planned, retired)"
                        ));
                    }
                };
                let uses = f
                    .uses
                    .into_iter()
                    .map(|u| match u {
                        UsesFront::Id(to) => Uses { to, why: None },
                        UsesFront::Full { to, why } => Uses { to, why },
                    })
                    .collect();
                self.architecture.insert(
                    id.clone(),
                    Element {
                        title: f.title.unwrap_or_else(title_fallback),
                        id,
                        level,
                        parent: f.parent.filter(|p| !p.is_empty()),
                        technology: f.technology,
                        paths: f.paths,
                        uses,
                        docs: f.docs,
                        state,
                        body: body.to_string(),
                        source,
                    },
                );
            }
            Kind::Question => {
                let f: QuestionFront = toml::from_str(front).map_err(|e| toml_msg(&e))?;
                let state = match f.state.as_deref().unwrap_or("open") {
                    "open" => QuestionState::Open,
                    "answered" => QuestionState::Answered,
                    other => {
                        return Err(format!("unknown question state `{other}` (open, answered)"));
                    }
                };
                if state == QuestionState::Answered
                    && f.answer.as_deref().is_none_or(|a| a.trim().is_empty())
                {
                    return Err("answered question has no `answer`".into());
                }
                let title = f.title.unwrap_or_else(title_fallback);
                self.questions.insert(
                    id.clone(),
                    Question {
                        id,
                        title,
                        state,
                        blocks: f.blocks,
                        answer: f.answer,
                        body: body.to_string(),
                        source,
                    },
                );
            }
        }
        Ok(())
    }

    /// Cross-references are checked after everything is loaded. A dangling
    /// reference is a problem, not a silent no-op: `checks = ["tset"]` must
    /// not quietly mean "no checks".
    fn validate(&mut self) {
        let mut found = Vec::new();
        for t in self.tasks.values() {
            for c in &t.checks {
                if !self.config.checks.contains_key(c) {
                    found.push((t.source.path.clone(), format!("unknown check `{c}`")));
                }
            }
            for a in &t.after {
                if !self.tasks.contains_key(a) {
                    found.push((
                        t.source.path.clone(),
                        format!("`after` names unknown task `{a}`"),
                    ));
                }
            }
        }
        for q in self.questions.values() {
            for b in &q.blocks {
                if !self.tasks.contains_key(b) {
                    found.push((
                        q.source.path.clone(),
                        format!("`blocks` names unknown task `{b}`"),
                    ));
                }
            }
        }
        for d in self.decisions.values() {
            for s in &d.supersedes {
                if !self.decisions.contains_key(s) {
                    found.push((
                        d.source.path.clone(),
                        format!("`supersedes` names unknown decision `{s}`"),
                    ));
                }
            }
        }
        let superseded = self.superseded();
        for m in self.memory.values() {
            for s in &m.supersedes {
                if s == &m.id {
                    found.push((
                        m.source.path.clone(),
                        "a note can't supersede itself".into(),
                    ));
                } else if !self.memory.contains_key(s) {
                    found.push((
                        m.source.path.clone(),
                        format!("`supersedes` names unknown note `{s}`"),
                    ));
                } else if self.memory[s].supersedes.contains(&m.id) && m.id < *s {
                    found.push((
                        m.source.path.clone(),
                        format!("notes `{}` and `{s}` supersede each other", m.id),
                    ));
                }
            }
        }
        let mut by_key: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for m in self.memory.values() {
            if let Some(k) = &m.key
                && m.state == MemoryState::Current
                && !superseded.contains_key(&m.id)
            {
                by_key.entry(k).or_default().push(&m.id);
            }
        }
        for (k, ids) in by_key {
            if ids.len() > 1 {
                let path = self.memory[ids[0]].source.path.clone();
                found.push((
                    path,
                    format!(
                        "notes {} all claim key `{k}`; neither wins until one supersedes the others",
                        ids.iter().map(|i| format!("`{i}`")).collect::<Vec<_>>().join(", ")
                    ),
                ));
            }
        }
        if let Some(cycle) = self.dependency_cycle() {
            let path = self.tasks[&cycle[0]].source.path.clone();
            found.push((path, format!("dependency cycle: {}", cycle.join(" -> "))));
        }
        for (path, detail) in found {
            self.problem(&path, detail);
        }
    }

    fn dependency_cycle(&self) -> Option<Vec<String>> {
        // Iterative DFS with colors; O(tasks + edges).
        #[derive(Clone, Copy, PartialEq)]
        enum Color {
            White,
            Grey,
            Black,
        }
        let mut color: BTreeMap<&str, Color> = self
            .tasks
            .keys()
            .map(|k| (k.as_str(), Color::White))
            .collect();
        for start in self.tasks.keys() {
            if color[start.as_str()] != Color::White {
                continue;
            }
            let mut stack: Vec<(&str, usize)> = vec![(start.as_str(), 0)];
            color.insert(start.as_str(), Color::Grey);
            while let Some((node, idx)) = stack.last().copied() {
                let deps = &self.tasks[node].after;
                if idx < deps.len() {
                    if let Some(frame) = stack.last_mut() {
                        frame.1 += 1;
                    }
                    let next = deps[idx].as_str();
                    match color.get(next) {
                        Some(Color::Grey) => {
                            let pos = stack.iter().position(|(n, _)| *n == next).unwrap_or(0);
                            let mut cycle: Vec<String> =
                                stack[pos..].iter().map(|(n, _)| n.to_string()).collect();
                            cycle.push(next.to_string());
                            return Some(cycle);
                        }
                        Some(Color::White) => {
                            color.insert(next, Color::Grey);
                            stack.push((next, 0));
                        }
                        _ => {}
                    }
                } else {
                    color.insert(node, Color::Black);
                    stack.pop();
                }
            }
        }
        None
    }

    pub fn protected(&self) -> Scope {
        let mut globs: Vec<String> = vec![format!("{DIR}/**")];
        globs.extend(HOOK_CONFIGS.iter().map(|p| p.to_string()));
        globs.extend(self.config.protect.globs().iter().cloned());
        Scope::new(globs)
    }

    /// Every entity id in one namespace, with its kind. Ids are unique per
    /// kind; the CLI resolves `kitsu show <id>` through this.
    pub fn find(&self, id: &str) -> Vec<(Kind, &Source)> {
        let mut out = Vec::new();
        if let Some(t) = self.tasks.get(id) {
            out.push((Kind::Task, &t.source));
        }
        if let Some(d) = self.decisions.get(id) {
            out.push((Kind::Decision, &d.source));
        }
        if let Some(q) = self.questions.get(id) {
            out.push((Kind::Question, &q.source));
        }
        out
    }

    pub fn problems_under(&self, prefix: &str) -> impl Iterator<Item = &Problem> {
        self.problems
            .iter()
            .filter(move |p| p.path.starts_with(prefix))
    }

    pub fn open_questions_blocking<'a>(
        &'a self,
        task: &'a str,
    ) -> impl Iterator<Item = &'a Question> {
        self.questions
            .values()
            .filter(move |q| q.state == QuestionState::Open && q.blocks.iter().any(|b| b == task))
    }

    /// Note id -> the current note that supersedes it. Derived on every
    /// read, so reverting the superseding commit brings the old note back.
    pub fn superseded(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for m in self
            .memory
            .values()
            .filter(|m| m.state == MemoryState::Current)
        {
            for s in &m.supersedes {
                if s != &m.id && self.memory.contains_key(s) {
                    out.entry(s.clone()).or_insert_with(|| m.id.clone());
                }
            }
        }
        out
    }

    pub fn ids(&self, kind: Kind) -> BTreeSet<String> {
        match kind {
            Kind::Task => self.tasks.keys().cloned().collect(),
            Kind::Decision => self.decisions.keys().cloned().collect(),
            Kind::Question => self.questions.keys().cloned().collect(),
            Kind::Memory => self.memory.keys().cloned().collect(),
            Kind::Architecture => self.architecture.keys().cloned().collect(),
        }
    }
}

/// The `(repo-relative path, contents)` pairs `Intent::load_dir` parses.
/// Exposed so an edit can be checked by parsing the result before it is
/// written.
pub fn dir_files(root: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    let base = root.join(DIR);
    if !base.exists() {
        return Ok(files);
    }
    let config = base.join("kitsu.toml");
    if config.exists() {
        let bytes =
            std::fs::read(&config).map_err(|e| Error::io(config.display().to_string(), e))?;
        files.push((CONFIG.to_string(), bytes));
    }
    // `invariants` is read only so a leftover file is reported.
    for sub in Kind::ALL.iter().map(|k| k.dir()).chain(["invariants"]) {
        let dir = base.join(sub);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(dir.display().to_string(), e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".md") || !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let bytes = std::fs::read(entry.path())
                .map_err(|e| Error::io(entry.path().display().to_string(), e))?;
            files.push((format!("{DIR}/{sub}/{name}"), bytes));
        }
    }
    Ok(files)
}

/// A memory note: protected like every `.kitsu/` file, but a proposal of
/// something learned rather than a change to what is required. Review
/// shows the two differently so reviewers don't learn to wave rule changes
/// through.
pub fn is_memory_note(path: &str) -> bool {
    matches!(classify(path), Some((Kind::Memory, _)))
}

/// `.kitsu/invariants/` is gone. A file still there is a rule that no
/// longer binds, so it is reported instead of skipped.
const RETIRED_INVARIANTS: &str = "invariants are no longer read: put `guards` (the paths it protects) and `why` on the check in kitsu.toml, and move the prose to a decision";

fn is_retired_invariant(path: &str) -> bool {
    path.strip_prefix(DIR)
        .and_then(|p| p.strip_prefix("/invariants/"))
        .is_some_and(|f| f.ends_with(".md") && !f.contains('/'))
}

pub fn classify(path: &str) -> Option<(Kind, &str)> {
    let rest = path.strip_prefix(DIR)?.strip_prefix('/')?;
    let (dir, file) = rest.split_once('/')?;
    if file.contains('/') {
        return None;
    }
    let kind = Kind::from_dir(dir)?;
    Some((kind, file.strip_suffix(".md")?))
}

pub fn valid_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && !id.starts_with('-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "`{id}` is not a valid id (lowercase letters, digits, `-`, `_`; max 64)"
        ))
    }
}

/// `+++\n<toml>\n+++\n<body>`. Front matter is required so a typo'd file is a
/// visible problem rather than a task with no constraints.
pub fn split_front_matter(text: &str) -> Result<(&str, &str), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("+++\n")
        .or_else(|| text.strip_prefix("+++\r\n"))
        .ok_or("missing `+++` front matter at the top of the file")?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "+++" {
            let front = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return Ok((front, body.trim_start_matches(['\n', '\r'])));
        }
        offset += line.len();
    }
    Err("front matter is not closed with `+++`".into())
}

/// Set `state = "<value>"` in a file's front matter, keeping everything
/// else byte for byte. Used when a human closes a task from the UI.
pub fn set_state(text: &str, value: &str) -> Result<String, String> {
    let (front, _) = split_front_matter(text)?;
    // `front` is a slice of `text`; its offset is the distance between them.
    let start = front.as_ptr() as usize - text.as_ptr() as usize;
    let end = start + front.len();
    let mut replaced = false;
    let mut new_front = String::with_capacity(front.len() + 20);
    for line in front.split_inclusive('\n') {
        if !replaced && line.trim_start().starts_with("state") && line.contains('=') {
            new_front.push_str(&format!("state = \"{value}\"\n"));
            replaced = true;
        } else {
            new_front.push_str(line);
        }
    }
    if !replaced {
        new_front.push_str(&format!("state = \"{value}\"\n"));
    }
    Ok(format!("{}{}{}", &text[..start], new_front, &text[end..]))
}

/// Set `key = [..]` in a file's front matter to `values`, keeping every
/// other byte of the file as it was. A multi-line array is replaced whole;
/// a comment on the same line as the value goes with it. A key that isn't
/// there is added only when there is something to say, before the first
/// table header so it stays top-level. Used when a person edits a task's
/// `after`, `checks` or `scope` from the UI.
pub fn set_list(text: &str, key: &str, values: &[String]) -> Result<String, String> {
    let (front, _) = split_front_matter(text)?;
    let start = front.as_ptr() as usize - text.as_ptr() as usize;
    let end = start + front.len();
    let (entries, tables_at) = front_entries(front);
    let list = values
        .iter()
        .map(|v| toml::Value::String(v.clone()).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let (at, eol) = match entries.iter().find(|(k, _)| k == key) {
        Some((_, range)) => {
            let old = &front[range.clone()];
            let eol = if old.ends_with("\r\n") {
                "\r\n"
            } else if old.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            (range.clone(), eol)
        }
        None if values.is_empty() => return Ok(text.to_string()),
        None => {
            let eol = if front.contains("\r\n") { "\r\n" } else { "\n" };
            (tables_at..tables_at, eol)
        }
    };
    let line = format!("{key} = [{list}]{eol}");
    let new_front = format!("{}{line}{}", &front[..at.start], &front[at.end..]);
    Ok(format!("{}{new_front}{}", &text[..start], &text[end..]))
}

/// Top-level `key = value` entries of TOML front matter, each as the byte
/// range from the start of its line to past the newline that ends its value
/// (so a multi-line array is one entry), plus where the first table header
/// starts (the end when there is none). Strings and comments are skipped,
/// so a `[` or newline inside them doesn't end anything early.
fn front_entries(front: &str) -> (Vec<(String, std::ops::Range<usize>)>, usize) {
    let b = front.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let line_end = |mut i: usize| {
        while i < b.len() && b[i] != b'\n' {
            i += 1;
        }
        (i + 1).min(b.len())
    };
    while i < b.len() {
        let line_start = i;
        while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        match b.get(i) {
            None => break,
            Some(b'[') => return (out, line_start),
            Some(b'#' | b'\n' | b'\r') => {
                i = line_end(i);
                continue;
            }
            _ => {}
        }
        let key_start = i;
        while i < b.len() && b[i] != b'=' && b[i] != b'\n' {
            i += 1;
        }
        if b.get(i) != Some(&b'=') {
            i = line_end(i);
            continue;
        }
        let key = front[key_start..i].trim().to_string();
        i += 1;
        let mut depth = 0i32;
        while i < b.len() {
            match b[i] {
                b'"' | b'\'' => {
                    i = skip_toml_string(b, i);
                    continue;
                }
                b'[' | b'{' => depth += 1,
                b']' | b'}' => depth -= 1,
                b'#' => {
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                b'\n' if depth <= 0 => {
                    i += 1;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        out.push((key, line_start..i));
    }
    (out, b.len())
}

/// Index just past the TOML string starting at `i` (basic, literal, or
/// either multi-line form). An unclosed string runs to the end.
fn skip_toml_string(b: &[u8], i: usize) -> usize {
    let q = b[i];
    let triple = b.len() >= i + 3 && b[i + 1] == q && b[i + 2] == q;
    let mut j = if triple { i + 3 } else { i + 1 };
    while j < b.len() {
        if q == b'"' && b[j] == b'\\' {
            j += 2;
            continue;
        }
        if b[j] == q {
            if !triple {
                return j + 1;
            }
            if b.len() >= j + 3 && b[j + 1] == q && b[j + 2] == q {
                // `"""a""""` closes on the last three quotes.
                let mut k = j + 3;
                while k < b.len() && b[k] == q && k < j + 5 {
                    k += 1;
                }
                return k;
            }
        }
        if !triple && b[j] == b'\n' {
            return j;
        }
        j += 1;
    }
    b.len()
}

fn first_heading(body: &str) -> Option<String> {
    body.lines()
        .find_map(|l| l.strip_prefix("# ").map(|h| h.trim().to_string()))
}

fn toml_msg(e: &toml::de::Error) -> String {
    // toml's messages are multi-line with a caret diagram; keep the first
    // meaningful line and the span so the UI can show it on one row.
    let msg = e.message().to_string();
    match e.span() {
        Some(span) => format!("{msg} (front matter bytes {}..{})", span.start, span.end),
        None => msg,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskFront {
    title: Option<String>,
    state: Option<String>,
    #[serde(default)]
    scope: Vec<String>,
    #[serde(default)]
    checks: Vec<String>,
    #[serde(default)]
    after: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryFront {
    title: Option<String>,
    kind: Option<String>,
    #[serde(default)]
    scope: Vec<String>,
    #[serde(default)]
    anchors: Vec<String>,
    by: Option<String>,
    run: Option<String>,
    state: Option<String>,
    #[serde(default)]
    supersedes: Vec<String>,
    key: Option<String>,
    reason: Option<String>,
}

/// Also used for personal notes outside the repository.
pub fn parse_memory(id: &str, front: &str, body: &str, source: Source) -> Result<Memory, String> {
    let f: MemoryFront = toml::from_str(front).map_err(|e| toml_msg(&e))?;
    let kind = match f.kind.as_deref() {
        None => {
            return Err(
                "memory note needs a `kind` (fact, gotcha, convention, preference, lesson)".into(),
            );
        }
        Some(k) => MemoryKind::parse(k).ok_or_else(|| {
            format!("unknown memory kind `{k}` (fact, gotcha, convention, preference, lesson)")
        })?,
    };
    if let Some(r) = &f.run {
        valid_id(r).map_err(|e| format!("`run` must be a run id: {e}"))?;
    }
    let state = match f.state.as_deref().unwrap_or("current") {
        "current" => MemoryState::Current,
        "retired" => MemoryState::Retired,
        other => return Err(format!("unknown memory state `{other}` (current, retired)")),
    };
    Ok(Memory {
        id: id.to_string(),
        title: f
            .title
            .unwrap_or_else(|| first_heading(body).unwrap_or_else(|| id.to_string())),
        kind,
        scope: Scope::new(f.scope),
        anchors: Scope::new(f.anchors),
        by: f.by,
        run: f.run,
        state,
        supersedes: f.supersedes,
        key: f.key.filter(|k| !k.trim().is_empty()),
        reason: f.reason,
        body: body.to_string(),
        source,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionFront {
    title: Option<String>,
    state: Option<String>,
    #[serde(default)]
    scope: Vec<String>,
    #[serde(default)]
    rejected: Vec<String>,
    #[serde(default)]
    supersedes: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionFront {
    title: Option<String>,
    state: Option<String>,
    #[serde(default)]
    blocks: Vec<String>,
    answer: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    checks: BTreeMap<String, CheckFile>,
    #[serde(default)]
    protect: ProtectFile,
    #[serde(default)]
    architecture: ArchitectureFile,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ArchitectureFile {
    #[serde(default)]
    cover: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ElementFront {
    title: Option<String>,
    level: String,
    parent: Option<String>,
    technology: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    uses: Vec<UsesFront>,
    #[serde(default)]
    docs: Vec<String>,
    state: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum UsesFront {
    Id(String),
    Full { to: String, why: Option<String> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckFile {
    run: String,
    timeout: Option<Timeout>,
    #[serde(default)]
    scope: Vec<String>,
    #[serde(default)]
    guards: Vec<String>,
    why: Option<String>,
    #[serde(default)]
    held_out: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ProtectFile {
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Timeout {
    Secs(u64),
    Text(String),
}

const DEFAULT_TIMEOUT_SECS: u64 = 600;

fn parse_config(text: &str) -> Result<Config, String> {
    let file: ConfigFile = toml::from_str(text).map_err(|e| toml_msg(&e))?;
    let mut checks = BTreeMap::new();
    for (name, c) in file.checks {
        valid_id(&name).map_err(|e| format!("check {e}"))?;
        if c.run.trim().is_empty() {
            return Err(format!("check `{name}` has an empty `run`"));
        }
        let timeout_secs = match c.timeout {
            None => DEFAULT_TIMEOUT_SECS,
            Some(Timeout::Secs(s)) => s,
            Some(Timeout::Text(t)) => parse_duration(&t)
                .ok_or_else(|| format!("check `{name}`: bad timeout `{t}` (try 90s, 10m, 1h)"))?,
        };
        if timeout_secs == 0 {
            return Err(format!("check `{name}`: timeout must be positive"));
        }
        checks.insert(
            name.clone(),
            CheckDef {
                name,
                run: c.run,
                timeout_secs,
                scope: Scope::new(c.scope),
                guards: Scope::new(c.guards),
                why: c
                    .why
                    .map(|w| w.trim().to_string())
                    .filter(|w| !w.is_empty()),
                held_out: c.held_out,
            },
        );
    }
    Ok(Config {
        checks,
        protect: Scope::new(file.protect.paths),
        architecture_cover: Scope::new(file.architecture.cover),
    })
}

fn parse_duration(t: &str) -> Option<u64> {
    let t = t.trim();
    let (num, mult) = match t.char_indices().last()? {
        (i, 's') => (&t[..i], 1),
        (i, 'm') => (&t[..i], 60),
        (i, 'h') => (&t[..i], 3600),
        _ => (t, 1),
    };
    num.trim().parse::<u64>().ok()?.checked_mul(mult)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
        list.iter()
            .map(|(p, c)| (p.to_string(), c.as_bytes().to_vec()))
            .collect()
    }

    const CONFIG_OK: &str = "[checks.test]\nrun = \"cargo test\"\ntimeout = \"2m\"\n\n[protect]\npaths = [\"tests/**\"]\n";

    #[test]
    fn parses_all_kinds() {
        let intent = Intent::from_files(files(&[
            (CONFIG, CONFIG_OK),
            (
                ".kitsu/tasks/retry.md",
                "+++\ntitle = \"Bound retries\"\nscope = [\"src/client/**\"]\nchecks = [\"test\"]\n+++\nBody here.\n",
            ),
            (
                ".kitsu/decisions/bounded.md",
                "+++\nrejected = [\"infinite retry\"]\n+++\n# Retry at most three times\n",
            ),
            (
                ".kitsu/questions/dedupe.md",
                "+++\ntitle = \"Does upstream dedupe?\"\nblocks = [\"retry\"]\n+++\n",
            ),
        ]));
        assert!(intent.problems.is_empty(), "{:?}", intent.problems);
        assert_eq!(intent.config.checks["test"].timeout_secs, 120);
        assert_eq!(intent.tasks["retry"].title, "Bound retries");
        assert_eq!(
            intent.decisions["bounded"].title,
            "Retry at most three times"
        );
        assert_eq!(intent.open_questions_blocking("retry").count(), 1);
        assert!(intent.protected().contains(".kitsu/decisions/bounded.md"));
        assert!(intent.protected().contains("tests/a.rs"));
    }

    #[test]
    fn broken_files_become_problems_not_silence() {
        let intent = Intent::from_files(files(&[
            (CONFIG, CONFIG_OK),
            (
                ".kitsu/invariants/typo.md",
                "+++\ntitle = \"x\"\nscpoe = [\"src/**\"]\n+++\n",
            ),
            (".kitsu/invariants/nofront.md", "just text"),
            (
                ".kitsu/tasks/a.md",
                "+++\nchecks = [\"tset\"]\nafter = [\"ghost\"]\n+++\n",
            ),
            (".kitsu/tasks/Bad Name.md", "+++\n+++\n"),
        ]));
        let msgs: Vec<String> = intent
            .problems
            .iter()
            .map(|p| format!("{}: {}", p.path, p.detail))
            .collect();
        assert_eq!(intent.problems.len(), 5, "{msgs:#?}");
        // A leftover invariant is a rule that stopped binding: loud, both
        // when it parses and when it doesn't.
        assert_eq!(
            msgs.iter()
                .filter(|m| m.contains("/invariants/") && m.contains("no longer read"))
                .count(),
            2,
            "{msgs:#?}"
        );
        assert!(msgs.iter().any(|m| m.contains("unknown check `tset`")));
        assert!(msgs.iter().any(|m| m.contains("unknown task `ghost`")));
    }

    #[test]
    fn detects_dependency_cycles() {
        let intent = Intent::from_files(files(&[
            (".kitsu/tasks/a.md", "+++\nafter = [\"b\"]\n+++\n"),
            (".kitsu/tasks/b.md", "+++\nafter = [\"c\"]\n+++\n"),
            (".kitsu/tasks/c.md", "+++\nafter = [\"a\"]\n+++\n"),
            (".kitsu/tasks/d.md", "+++\nafter = [\"a\"]\n+++\n"),
        ]));
        assert_eq!(intent.problems.len(), 1);
        assert!(
            intent.problems[0].detail.contains("cycle"),
            "{:?}",
            intent.problems
        );
    }

    #[test]
    fn answered_question_needs_an_answer() {
        let intent = Intent::from_files(files(&[(
            ".kitsu/questions/q.md",
            "+++\nstate = \"answered\"\n+++\n",
        )]));
        assert_eq!(intent.problems.len(), 1);
    }

    #[test]
    fn checks_carry_what_they_guard_and_why() {
        let intent = Intent::from_files(files(&[(
            CONFIG,
            "[checks.idem]\nrun = \"pytest\"\nguards = [\"payments.py\"]\nwhy = \"\"\"\nOne key per charge.\n\"\"\"\n[checks.lint]\nrun = \"ruff\"\nwhy = \"  \"\n",
        )]));
        assert!(intent.problems.is_empty(), "{:?}", intent.problems);
        let idem = &intent.config.checks["idem"];
        assert_eq!(idem.guards.globs(), ["payments.py"]);
        assert_eq!(idem.why.as_deref(), Some("One key per charge."));
        let lint = &intent.config.checks["lint"];
        assert!(lint.guards.is_everything() && lint.why.is_none());
        assert!(!idem.held_out && !lint.held_out);
    }

    #[test]
    fn held_out_checks_and_hook_configs() {
        let intent = Intent::from_files(files(&[(
            CONFIG,
            "[checks.hidden]\nrun = \"sh \\\"$KITSU_HELD_OUT/run.sh\\\"\"\nguards = [\"src/**\"]\nheld_out = true\n",
        )]));
        assert!(intent.problems.is_empty(), "{:?}", intent.problems);
        let c = &intent.config.checks["hidden"];
        assert!(c.held_out);
        assert!(!c.shown_command().contains("KITSU_HELD_OUT"));
        // The agents' hook configs decide whether the gate runs: rules.
        for p in HOOK_CONFIGS {
            assert!(intent.protected().contains(p), "{p}");
        }
        assert!(!intent.protected().contains(".claude/agents/x.md"));
    }

    #[test]
    fn check_fingerprint_tracks_command_not_scope() {
        let a = CheckDef {
            name: "t".into(),
            run: "cargo test".into(),
            timeout_secs: 60,
            scope: Scope::new(["a/**"]),
            guards: Scope::new(["a/**"]),
            why: None,
            held_out: false,
        };
        let mut b = a.clone();
        b.guards = Scope::new(["c/**"]);
        b.scope = Scope::new(["b/**"]);
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.run = "cargo test --all".into();
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn set_state_edits_only_that_line() {
        let t = "+++\ntitle = \"x\"\nstate = \"open\"\nscope = []\n+++\nbody\n";
        assert_eq!(
            set_state(t, "done").expect("edit"),
            "+++\ntitle = \"x\"\nstate = \"done\"\nscope = []\n+++\nbody\n"
        );
        let t = "+++\ntitle = \"x\"\n+++\nbody\n";
        assert_eq!(
            set_state(t, "done").expect("edit"),
            "+++\ntitle = \"x\"\nstate = \"done\"\n+++\nbody\n"
        );
        let t = "+++\n+++\n";
        assert_eq!(
            set_state(t, "done").expect("edit"),
            "+++\nstate = \"done\"\n+++\n"
        );
    }

    #[test]
    fn set_list_rewrites_only_that_key() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // One-line value, other keys and the body untouched byte for byte.
        let t = "+++\ntitle = \"x [y]\"   # keep = me\nscope = [\"a/**\"]\nafter = [\"b\"]\n+++\nbody\nafter = [\"not front matter\"]\n";
        assert_eq!(
            set_list(t, "after", &v(&["b", "c"])).expect("edit"),
            "+++\ntitle = \"x [y]\"   # keep = me\nscope = [\"a/**\"]\nafter = [\"b\", \"c\"]\n+++\nbody\nafter = [\"not front matter\"]\n"
        );
        // A multi-line array with comments and brackets inside strings is one
        // entry; the next key survives.
        let t = "+++\nchecks = [\n  \"test\", # the suite\n  \"lint]\",\n]\nafter = []\n+++\n";
        assert_eq!(
            set_list(t, "checks", &v(&["fmt"])).expect("edit"),
            "+++\nchecks = [\"fmt\"]\nafter = []\n+++\n"
        );
        // A multi-line string that mentions the key is not the key.
        let t = "+++\ntitle = \"\"\"\nafter = [\"x\"]\n\"\"\"\n+++\n";
        assert_eq!(
            set_list(t, "after", &v(&["y"])).expect("edit"),
            "+++\ntitle = \"\"\"\nafter = [\"x\"]\n\"\"\"\nafter = [\"y\"]\n+++\n"
        );
        // Missing key: added only if there's something to add.
        let t = "+++\ntitle = \"x\"\n+++\nbody\n";
        assert_eq!(set_list(t, "after", &[]).expect("edit"), t);
        assert_eq!(
            set_list(t, "after", &v(&["a"])).expect("edit"),
            "+++\ntitle = \"x\"\nafter = [\"a\"]\n+++\nbody\n"
        );
        assert_eq!(
            set_list("+++\n+++\n", "scope", &v(&["src/\"q\"/**"])).expect("edit"),
            "+++\nscope = ['src/\"q\"/**']\n+++\n"
        );
        // Emptying keeps the key; CRLF files stay CRLF.
        let t = "+++\r\ntitle = \"x\"\r\nafter = [\"a\"]\r\n+++\r\nbody\r\n";
        assert_eq!(
            set_list(t, "after", &[]).expect("edit"),
            "+++\r\ntitle = \"x\"\r\nafter = []\r\n+++\r\nbody\r\n"
        );
        // A table header ends the top level: new keys go before it.
        let t = "+++\ntitle = \"x\"\n[extra]\nafter = 1\n+++\n";
        assert_eq!(
            set_list(t, "after", &v(&["a"])).expect("edit"),
            "+++\ntitle = \"x\"\nafter = [\"a\"]\n[extra]\nafter = 1\n+++\n"
        );
        assert!(set_list("no front matter", "after", &[]).is_err());
    }

    #[test]
    fn set_list_output_parses_to_what_was_asked() {
        let t = "+++\ntitle = \"Retry\"\nstate = \"open\"\nscope = [\"src/**\"]\nchecks = [\n  \"test\",\n]\n+++\n# Retry\n\nWhy.\n";
        let mut out = set_list(t, "after", &["b".into()]).expect("after");
        out = set_list(&out, "checks", &["test".into(), "lint".into()]).expect("checks");
        out = set_list(&out, "scope", &[]).expect("scope");
        let intent = Intent::from_files(files(&[
            (
                CONFIG,
                "[checks.test]\nrun = \"t\"\n[checks.lint]\nrun = \"l\"\n",
            ),
            (".kitsu/tasks/a.md", &out),
            (".kitsu/tasks/b.md", "+++\n+++\n"),
        ]));
        assert!(intent.problems.is_empty(), "{:?}", intent.problems);
        let a = &intent.tasks["a"];
        assert_eq!(a.after, ["b"]);
        assert_eq!(a.checks, ["test", "lint"]);
        assert!(a.scope.is_everything());
        assert_eq!(a.title, "Retry");
        assert_eq!(a.body, "# Retry\n\nWhy.\n");
    }

    #[test]
    fn front_matter_edge_cases() {
        assert!(split_front_matter("+++\r\ntitle = \"x\"\r\n+++\r\nbody").is_ok());
        assert!(split_front_matter("\u{feff}+++\n+++\n").is_ok());
        assert!(split_front_matter("+++\ntitle = 1\n").is_err());
        assert_eq!(parse_duration("10m"), Some(600));
        assert_eq!(parse_duration("45"), Some(45));
        assert_eq!(parse_duration("abc"), None);
    }
}
