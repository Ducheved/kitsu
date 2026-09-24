//! Intent: the part of engineering state that people review.
//!
//! Tasks, decisions, invariants and open questions live as Markdown files
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Task,
    Decision,
    Invariant,
    Question,
    Memory,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Task,
        Kind::Decision,
        Kind::Invariant,
        Kind::Question,
        Kind::Memory,
    ];

    pub fn dir(self) -> &'static str {
        match self {
            Kind::Task => "tasks",
            Kind::Decision => "decisions",
            Kind::Invariant => "invariants",
            Kind::Question => "questions",
            Kind::Memory => "memory",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Task => "task",
            Kind::Decision => "decision",
            Kind::Invariant => "invariant",
            Kind::Question => "question",
            Kind::Memory => "memory",
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
}

impl CheckDef {
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
pub enum InvariantState {
    Active,
    Retired,
}

#[derive(Debug, Clone)]
pub struct Invariant {
    pub id: String,
    pub title: String,
    pub state: InvariantState,
    pub scope: Scope,
    pub checks: Vec<String>,
    pub decision: Option<String>,
    pub body: String,
    pub source: Source,
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
/// broken invariant file means a constraint is missing, and the brief and the
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
    pub invariants: BTreeMap<String, Invariant>,
    pub questions: BTreeMap<String, Question>,
    pub memory: BTreeMap<String, Memory>,
    pub problems: Vec<Problem>,
}

impl Intent {
    /// Read `.kitsu/` from a working directory.
    pub fn load_dir(root: &Path) -> Result<Intent> {
        let mut files = Vec::new();
        let base = root.join(DIR);
        if !base.exists() {
            return Ok(Intent::default());
        }
        let config = base.join("kitsu.toml");
        if config.exists() {
            let bytes =
                std::fs::read(&config).map_err(|e| Error::io(config.display().to_string(), e))?;
            files.push((CONFIG.to_string(), bytes));
        }
        for kind in Kind::ALL {
            let dir = base.join(kind.dir());
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries {
                let entry = entry.map_err(|e| Error::io(dir.display().to_string(), e))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".md")
                    || !entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                {
                    continue;
                }
                let bytes = std::fs::read(entry.path())
                    .map_err(|e| Error::io(entry.path().display().to_string(), e))?;
                files.push((format!("{DIR}/{}/{name}", kind.dir()), bytes));
            }
        }
        Ok(Intent::from_files(files))
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
            Kind::Invariant => {
                let f: InvariantFront = toml::from_str(front).map_err(|e| toml_msg(&e))?;
                let state = match f.state.as_deref().unwrap_or("active") {
                    "active" => InvariantState::Active,
                    "retired" => InvariantState::Retired,
                    other => {
                        return Err(format!(
                            "unknown invariant state `{other}` (active, retired)"
                        ));
                    }
                };
                let title = f.title.unwrap_or_else(title_fallback);
                self.invariants.insert(
                    id.clone(),
                    Invariant {
                        id,
                        title,
                        state,
                        scope: Scope::new(f.scope),
                        checks: f.checks,
                        decision: f.decision,
                        body: body.to_string(),
                        source,
                    },
                );
            }
            Kind::Memory => {
                let m = parse_memory(&id, front, body, source)?;
                self.memory.insert(id, m);
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
        for i in self.invariants.values() {
            for c in &i.checks {
                if !self.config.checks.contains_key(c) {
                    found.push((i.source.path.clone(), format!("unknown check `{c}`")));
                }
            }
            if let Some(d) = &i.decision
                && !self.decisions.contains_key(d)
            {
                found.push((i.source.path.clone(), format!("unknown decision `{d}`")));
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
        if let Some(i) = self.invariants.get(id) {
            out.push((Kind::Invariant, &i.source));
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
            Kind::Invariant => self.invariants.keys().cloned().collect(),
            Kind::Question => self.questions.keys().cloned().collect(),
            Kind::Memory => self.memory.keys().cloned().collect(),
        }
    }
}

/// A memory note: protected like every `.kitsu/` file, but a proposal of
/// something learned rather than a change to what is required. Review
/// shows the two differently so reviewers don't learn to wave rule changes
/// through.
pub fn is_memory_note(path: &str) -> bool {
    matches!(classify(path), Some((Kind::Memory, _)))
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
struct InvariantFront {
    title: Option<String>,
    state: Option<String>,
    #[serde(default)]
    scope: Vec<String>,
    #[serde(default)]
    checks: Vec<String>,
    decision: Option<String>,
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckFile {
    run: String,
    timeout: Option<Timeout>,
    #[serde(default)]
    scope: Vec<String>,
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
            },
        );
    }
    Ok(Config {
        checks,
        protect: Scope::new(file.protect.paths),
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
                ".kitsu/invariants/idem.md",
                "+++\ntitle = \"Stable idempotency key\"\nscope = [\"src/client/**\"]\nchecks = [\"test\"]\ndecision = \"bounded\"\n+++\n",
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
        assert_eq!(
            intent.invariants["idem"].decision.as_deref(),
            Some("bounded")
        );
        assert_eq!(intent.open_questions_blocking("retry").count(), 1);
        assert!(intent.protected().contains(".kitsu/invariants/idem.md"));
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
        assert!(
            msgs.iter()
                .any(|m| m.contains("typo.md") && m.contains("scpoe"))
        );
        assert!(msgs.iter().any(|m| m.contains("unknown check `tset`")));
        assert!(msgs.iter().any(|m| m.contains("unknown task `ghost`")));
        assert!(!intent.invariants.contains_key("typo"));
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
    fn check_fingerprint_tracks_command_not_scope() {
        let a = CheckDef {
            name: "t".into(),
            run: "cargo test".into(),
            timeout_secs: 60,
            scope: Scope::new(["a/**"]),
        };
        let mut b = a.clone();
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
    fn front_matter_edge_cases() {
        assert!(split_front_matter("+++\r\ntitle = \"x\"\r\n+++\r\nbody").is_ok());
        assert!(split_front_matter("\u{feff}+++\n+++\n").is_ok());
        assert!(split_front_matter("+++\ntitle = 1\n").is_err());
        assert_eq!(parse_duration("10m"), Some(600));
        assert_eq!(parse_duration("45"), Some(45));
        assert_eq!(parse_duration("abc"), None);
    }
}
