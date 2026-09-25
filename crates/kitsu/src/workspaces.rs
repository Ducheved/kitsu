//! The projects the desktop window shows side by side.
//!
//! ```text
//! <config_dir>/workspaces.toml    roots, display names, order
//! <config_dir>/workspaces.lock    held around every read-modify-write
//! ```
//!
//! Only the list lives here. Everything about one repository stays where it
//! always was: intent in `.kitsu/`, execution state under the git common
//! dir, trust in `<config_dir>/trusted`. Adding a repository doesn't trust
//! it, and removing one never touches its files.
//!
//! The file is written by the app and by `kitsu workspaces add|remove`. A
//! hand edit is fine, but a file that doesn't parse is reported, never
//! rewritten: overwriting it would drop whatever the edit meant.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::git::{BranchInfo, Git};
use crate::intent::{self, Intent};
use crate::status::{Attention, Snapshot};
use crate::util::{sha256_hex, short_id};
use crate::workspace::{Workspace, config_dir};

pub const FILE: &str = "workspaces.toml";
const MAX_NAME: usize = 64;

const HEADER: &str = "\
# Projects Kitsu's window shows, in this order. Written by the app and by
# `kitsu workspaces add|remove`; each repository keeps its own state in
# .kitsu/ and under its git dir. Removing an entry never touches its files.
";

/// One repository in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The main worktree, absolute and canonical when Kitsu wrote it.
    pub root: PathBuf,
    /// Shown instead of the folder name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Entry {
    /// Stable handle for the window: derived from the root, so it's the same
    /// across restarts and never a path the webview could make up.
    pub fn id(&self) -> String {
        id_of(&self.root)
    }

    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| folder_name(&self.root))
    }
}

pub fn id_of(root: &Path) -> String {
    sha256_hex(root.display().to_string().as_bytes())[..12].to_string()
}

fn folder_name(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Doc {
    #[serde(default, rename = "workspace")]
    workspaces: Vec<Entry>,
}

/// Parse and validate the file's text. Refuses what would make the list
/// ambiguous: relative roots, the same root twice, blank or oversized names.
pub fn parse(text: &str, path: &Path) -> Result<Vec<Entry>> {
    let bad = |detail: String| Error::Parse {
        path: path.to_path_buf(),
        detail,
    };
    let doc: Doc = toml::from_str(text).map_err(|e| bad(e.message().to_string()))?;
    let mut seen = BTreeSet::new();
    for e in &doc.workspaces {
        if !e.root.is_absolute() {
            return Err(bad(format!(
                "root must be an absolute path, got {}",
                e.root.display()
            )));
        }
        if !seen.insert(e.root.clone()) {
            return Err(bad(format!("{} is listed twice", e.root.display())));
        }
        if let Some(n) = &e.name {
            check_name(n).map_err(|d| bad(format!("{}: {d}", e.root.display())))?;
        }
    }
    Ok(doc.workspaces)
}

fn check_name(name: &str) -> std::result::Result<(), String> {
    if name.trim().is_empty() {
        return Err("name is empty".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("name is longer than {MAX_NAME} characters"));
    }
    if name.chars().any(char::is_control) {
        return Err("name has control characters".into());
    }
    Ok(())
}

pub fn render(entries: &[Entry]) -> Result<String> {
    let doc = Doc {
        workspaces: entries.to_vec(),
    };
    let body = toml::to_string(&doc).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(format!("{HEADER}\n{body}"))
}

/// The list at one path. The app and the CLI use `List::default_path()`;
/// tests pass their own so they never see the user's.
#[derive(Debug, Clone)]
pub struct List {
    path: PathBuf,
}

impl List {
    pub fn new(path: impl Into<PathBuf>) -> List {
        List { path: path.into() }
    }

    pub fn default_path() -> PathBuf {
        config_dir().join(FILE)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A missing file is an empty list.
    pub fn load(&self) -> Result<Vec<Entry>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => parse(&text, &self.path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(Error::io(self.path.display().to_string(), e)),
        }
    }

    pub fn find(&self, id: &str) -> Result<Option<Entry>> {
        Ok(self.load()?.into_iter().find(|e| e.id() == id))
    }

    /// Add the repository at `folder` (its main worktree, if `folder` is a
    /// linked one). Refused: a folder that isn't a git repository, a folder
    /// inside one rather than its top, and a repository already listed.
    /// Returns the entry and whether it was new; adding what's already
    /// there with `ok_if_listed` is not an error (the app does that for the
    /// folder it was started in).
    pub fn add(
        &self,
        folder: &Path,
        name: Option<&str>,
        ok_if_listed: bool,
    ) -> Result<(Entry, bool)> {
        let root = resolve(folder)?;
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(String::from);
        if let Some(n) = &name {
            check_name(n).map_err(Error::Invalid)?;
        }
        self.edit(|list| {
            if let Some(e) = list.iter().find(|e| same_dir(&e.root, &root)) {
                if ok_if_listed {
                    return Ok((e.clone(), false));
                }
                let how = if same_dir(folder, &root) {
                    String::new()
                } else {
                    format!(" ({} is one of its worktrees)", folder.display())
                };
                return Err(Error::Conflict(format!(
                    "{} is already in the list as {}{how}",
                    root.display(),
                    e.display_name()
                )));
            }
            let e = Entry { root, name };
            list.push(e.clone());
            Ok((e, true))
        })
    }

    /// Take a repository off the list. Its files, `.kitsu/`, runs and trust
    /// stay exactly as they are.
    pub fn remove(&self, id: &str) -> Result<Entry> {
        self.edit(|list| {
            let i = position(list, id)?;
            Ok(list.remove(i))
        })
    }

    /// `None` or a blank name goes back to the folder name.
    pub fn rename(&self, id: &str, name: Option<&str>) -> Result<Entry> {
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(String::from);
        if let Some(n) = &name {
            check_name(n).map_err(Error::Invalid)?;
        }
        self.edit(|list| {
            let i = position(list, id)?;
            list[i].name = name;
            Ok(list[i].clone())
        })
    }

    /// Move an entry to `index` (clamped to the end).
    pub fn reorder(&self, id: &str, index: usize) -> Result<()> {
        self.edit(|list| {
            let i = position(list, id)?;
            let e = list.remove(i);
            list.insert(index.min(list.len()), e);
            Ok(())
        })
    }

    /// Read, change and write under the lock, so the app and a terminal
    /// `kitsu workspaces add` can't lose each other's edit. A file that
    /// doesn't parse fails the edit before anything is written.
    fn edit<T>(&self, f: impl FnOnce(&mut Vec<Entry>) -> Result<T>) -> Result<T> {
        let _lock = self.lock()?;
        let mut list = self.load()?;
        let out = f(&mut list)?;
        let text = render(&list)?;
        let dir = self.dir();
        let tmp = dir.join(format!(".{FILE}.{}", short_id('w')));
        std::fs::write(&tmp, text).map_err(|e| Error::io(tmp.display().to_string(), e))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| Error::io(self.path.display().to_string(), e))?;
        Ok(out)
    }

    fn dir(&self) -> PathBuf {
        self.path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn lock(&self) -> Result<File> {
        let dir = self.dir();
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(dir.display().to_string(), e))?;
        let path = self.path.with_extension("lock");
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| Error::io(path.display().to_string(), e))?;
        f.lock()
            .map_err(|e| Error::io(format!("locking {}", path.display()), e))?;
        Ok(f)
    }
}

fn position(list: &[Entry], id: &str) -> Result<usize> {
    list.iter()
        .position(|e| e.id() == id)
        .ok_or_else(|| Error::NotFound(format!("project {id} is not in the list")))
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    canonical(a) == canonical(b)
}

/// The main worktree of the repository whose top level is `folder`.
pub fn resolve(folder: &Path) -> Result<PathBuf> {
    let meta = std::fs::metadata(folder).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            Error::NotFound(format!("no folder at {}", folder.display()))
        }
        _ => Error::io(folder.display().to_string(), e),
    })?;
    if !meta.is_dir() {
        return Err(Error::Invalid(format!(
            "{} is a file, not a folder",
            folder.display()
        )));
    }
    let folder = canonical(folder);
    let not_a_repo = || {
        Error::Invalid(format!(
            "{} isn't a git repository. Kitsu keeps its state next to git's: run `git init` there first",
            folder.display()
        ))
    };
    let git = Git::new(&folder);
    let top = match git.toplevel() {
        Ok(t) => canonical(&t),
        Err(Error::Git { .. }) => return Err(not_a_repo()),
        Err(e) => return Err(e),
    };
    if top != folder {
        return Err(Error::Invalid(format!(
            "{} is inside the repository {}, not a repository of its own. Add {} instead",
            folder.display(),
            top.display(),
            top.display()
        )));
    }
    let ws = Workspace::discover(&folder).map_err(|e| match e {
        Error::NotFound(_) => not_a_repo(),
        e => e,
    })?;
    Ok(canonical(&ws.root))
}

/// What the switcher shows for one repository: enough to decide where to
/// look, cheap enough to poll.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub root: String,
    pub branch: Option<String>,
    pub trusted: bool,
    pub initialized: bool,
    pub needs_you: usize,
    pub working: usize,
    pub ready: usize,
    /// The folder isn't there any more (moved, deleted, an unmounted disk).
    pub missing: bool,
    /// Why the counts are missing: the folder is gone, `.kitsu/` doesn't
    /// parse, the state database is locked... Counts are 0 then, not a guess.
    pub error: Option<String>,
}

/// Counts for one repository, derived from the same `status::Snapshot` as
/// the task list. Reads only: nothing from the repository runs, so it's
/// safe on an untrusted one.
pub fn summarize(e: &Entry) -> Summary {
    let mut s = Summary {
        id: e.id(),
        name: e.display_name(),
        root: e.root.display().to_string(),
        branch: None,
        trusted: false,
        initialized: false,
        needs_you: 0,
        working: 0,
        ready: 0,
        missing: false,
        error: None,
    };
    if !e.root.is_dir() {
        s.missing = true;
        s.error = Some(format!("{} is gone", e.root.display()));
        return s;
    }
    if let Err(err) = fill(e, &mut s) {
        s.error = Some(err.to_string());
    }
    s
}

fn fill(e: &Entry, s: &mut Summary) -> Result<()> {
    let ws = Workspace::open(&e.root)?;
    s.trusted = ws.is_trusted()?;
    s.branch = ws.current_branch()?;
    s.initialized = ws.root.join(intent::DIR).is_dir();
    if !s.initialized {
        return Ok(());
    }
    let intent = Intent::load_dir(&ws.root)?;
    let store = ws.open_store()?;
    let git = ws.git();
    let tasks = Snapshot {
        intent: &intent,
        git: &git,
        store: &store,
    }
    .tasks()?;
    for t in &tasks {
        match t.attention {
            Attention::NeedsYou => s.needs_you += 1,
            Attention::Working => s.working += 1,
            Attention::Ready => s.ready += 1,
            _ => {}
        }
    }
    Ok(())
}

/// Changes whenever something a summary or the window reads could have:
/// the rule files, HEAD and the index, the state database. Costs a few
/// dozen `stat`s and no process, so it can run every 250 ms per repository.
pub fn fingerprint(ws: &Workspace) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut stat = |p: &Path| {
        let m = std::fs::metadata(p).ok();
        m.as_ref().and_then(|m| m.modified().ok()).hash(&mut h);
        m.map(|m| m.len()).hash(&mut h);
    };
    let base = ws.root.join(intent::DIR);
    stat(&base.join("kitsu.toml"));
    for kind in intent::Kind::ALL {
        let dir = base.join(kind.dir());
        stat(&dir);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                stat(&e.path());
            }
        }
    }
    let git = ws.root.join(".git");
    stat(&git.join("HEAD"));
    stat(&git.join("index"));
    // WAL mode: a commit appends to -wal; a checkpoint rewrites the main file.
    stat(&ws.state.join("state.db"));
    stat(&ws.state.join("state.db-wal"));
    h.finish()
}

/// Summaries from the last call, reused while a repository's fingerprint
/// hasn't moved. The window polls the switcher's counts; with nothing
/// changed that poll costs `stat`s, not a SQLite open per repository.
#[derive(Default)]
pub struct Cache {
    seen: std::sync::Mutex<std::collections::HashMap<PathBuf, (u64, std::time::Instant, Summary)>>,
}

/// Recompute anyway after this long, in case a change slipped past the
/// fingerprint (a coarse filesystem clock, say).
const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30);

impl Cache {
    pub fn overview(&self, entries: &[Entry]) -> Vec<Summary> {
        // Trust lives outside the repository; if the file moved, redo all.
        let trust = std::fs::metadata(config_dir().join("trusted"))
            .and_then(|m| m.modified())
            .ok();
        let keys: Vec<u64> = entries
            .iter()
            .map(|e| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                trust.hash(&mut h);
                e.root.is_dir().hash(&mut h);
                if let Ok(ws) = Workspace::open(&e.root) {
                    fingerprint(&ws).hash(&mut h);
                }
                h.finish()
            })
            .collect();
        let mut out: Vec<Option<Summary>> = {
            let seen = self.seen.lock().unwrap_or_else(|p| p.into_inner());
            entries
                .iter()
                .zip(&keys)
                .map(|(e, k)| {
                    seen.get(&e.root)
                        .filter(|(key, at, _)| key == k && at.elapsed() < MAX_AGE)
                        .map(|(_, _, s)| Summary {
                            name: e.display_name(),
                            ..s.clone()
                        })
                })
                .collect()
        };
        let stale: Vec<usize> = (0..entries.len()).filter(|&i| out[i].is_none()).collect();
        let fresh = overview(
            &stale
                .iter()
                .map(|&i| entries[i].clone())
                .collect::<Vec<_>>(),
        );
        let mut seen = self.seen.lock().unwrap_or_else(|p| p.into_inner());
        let now = std::time::Instant::now();
        for (i, s) in stale.into_iter().zip(fresh) {
            seen.insert(entries[i].root.clone(), (keys[i], now, s.clone()));
            out[i] = Some(s);
        }
        seen.retain(|root, _| entries.iter().any(|e| &e.root == root));
        out.into_iter().flatten().collect()
    }
}

/// Every entry, summarized in parallel: each one is a few file reads, a
/// SQLite open and a query or two, so the whole list costs about as much
/// as its slowest repository.
pub fn overview(entries: &[Entry]) -> Vec<Summary> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = entries
            .iter()
            .map(|e| scope.spawn(move || summarize(e)))
            .collect();
        handles
            .into_iter()
            .zip(entries)
            .map(|(h, e)| {
                h.join().unwrap_or_else(|_| Summary {
                    error: Some("summary thread panicked".into()),
                    ..summarize_gone(e)
                })
            })
            .collect()
    })
}

fn summarize_gone(e: &Entry) -> Summary {
    Summary {
        id: e.id(),
        name: e.display_name(),
        root: e.root.display().to_string(),
        branch: None,
        trusted: false,
        initialized: false,
        needs_you: 0,
        working: 0,
        ready: 0,
        missing: false,
        error: None,
    }
}

/// Whose a worktree is.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Owner {
    /// The main worktree: where you work and where rules are read.
    Main,
    /// A Kitsu run's isolated checkout.
    Run {
        run: String,
        task: Option<String>,
        state: Option<String>,
    },
    /// Kitsu's scratch checkout for building an accept.
    Integration,
    /// One you made yourself.
    Yours,
}

#[derive(Debug, Clone, Serialize)]
pub struct Worktree {
    pub path: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub locked: bool,
    pub prunable: bool,
    pub owner: Owner,
}

#[derive(Debug, Clone, Serialize)]
pub struct Branch {
    #[serde(flatten)]
    pub info: BranchInfo,
    /// Set for `kitsu/run/<id>` branches.
    pub run: Option<String>,
    pub task: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitView {
    pub branch: Option<String>,
    pub branches: Vec<Branch>,
    pub worktrees: Vec<Worktree>,
}

pub const RUN_BRANCH_PREFIX: &str = "kitsu/run/";

/// Branches and worktrees of one repository, each marked as yours or as a
/// Kitsu run's (with its task). Read-only.
pub fn git_view(ws: &Workspace) -> Result<GitView> {
    let git = ws.git();
    let listed = {
        // `worktree list` reads every worktree's metadata; a concurrent
        // `worktree add` can make it fail halfway. Same lock the runs take.
        let _guard = ws.lock_worktrees()?;
        git.worktrees()?
    };
    let store = ws.open_store()?;
    let run_info = |id: &str| store.run(id).ok().map(|r| (r.task, r.state));
    let root = canonical(&ws.root);
    let runs = canonical(&ws.state.join("worktrees"));
    let integrate = canonical(&ws.state.join("integrate"));
    let worktrees = listed
        .into_iter()
        .filter(|w| !w.bare)
        .map(|w| {
            let path = canonical(&w.path);
            let owner = if path == root {
                Owner::Main
            } else if let Ok(rel) = path.strip_prefix(&runs) {
                let run = rel
                    .components()
                    .next()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .unwrap_or_default();
                let info = run_info(&run);
                Owner::Run {
                    task: info.as_ref().map(|i| i.0.clone()),
                    state: info.map(|i| i.1.as_str().to_string()),
                    run,
                }
            } else if path.starts_with(&integrate) {
                Owner::Integration
            } else {
                Owner::Yours
            };
            Worktree {
                path: w.path.display().to_string(),
                head: w.head,
                branch: w.branch,
                locked: w.locked,
                prunable: w.prunable,
                owner,
            }
        })
        .collect();
    let branches: Vec<Branch> = git
        .branches()?
        .into_iter()
        .map(|info| {
            let run = info.name.strip_prefix(RUN_BRANCH_PREFIX).map(String::from);
            let task = run.as_deref().and_then(run_info).map(|i| i.0);
            Branch { info, run, task }
        })
        .collect();
    Ok(GitView {
        branch: branches
            .iter()
            .find(|b| b.info.current)
            .map(|b| b.info.name.clone()),
        branches,
        worktrees,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::TempRepo;

    fn temp_list() -> (List, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kitsu-ws-{}", short_id('t')));
        std::fs::create_dir_all(&dir).expect("dir");
        (List::new(dir.join(FILE)), dir)
    }

    /// `/a` is absolute on Unix only; `C:/a` is on Windows.
    fn abs(p: &str) -> String {
        if cfg!(windows) {
            format!("C:{p}")
        } else {
            p.to_string()
        }
    }

    #[test]
    fn parse_refuses_what_would_make_the_list_ambiguous() {
        let p = Path::new("/cfg/workspaces.toml");
        let (a, b) = (abs("/a"), abs("/b"));
        let ok = parse(
            &format!(
                "[[workspace]]\nroot = \"{a}\"\n\n[[workspace]]\nroot = \"{b}\"\nname = \"Bee\"\n"
            ),
            p,
        )
        .expect("ok");
        assert_eq!(ok.len(), 2);
        assert_eq!(ok[0].display_name(), "a");
        assert_eq!(ok[1].display_name(), "Bee");
        assert!(parse("", p).expect("empty").is_empty());

        let mut refused = vec![
            (
                "[[workspace]]\nroot = \"rel/path\"\n".to_string(),
                "absolute",
            ),
            (
                format!("[[workspace]]\nroot = \"{a}\"\n[[workspace]]\nroot = \"{a}\"\n"),
                "twice",
            ),
            (
                format!("[[workspace]]\nroot = \"{a}\"\nname = \"  \"\n"),
                "empty",
            ),
            (
                format!("[[workspace]]\nroot = \"{a}\"\ncolour = \"red\"\n"),
                "unknown field",
            ),
            ("[[workspace]]\nname = \"x\"\n".to_string(), "root"),
            ("[[workspace]\n".to_string(), ""),
            ("theme = \"dark\"\n".to_string(), "unknown field"),
        ];
        if cfg!(windows) {
            // Rooted on whatever the current drive is, or relative to a
            // drive's current folder: neither names one folder.
            refused.push(("[[workspace]]\nroot = \"/a\"\n".to_string(), "absolute"));
            refused.push(("[[workspace]]\nroot = 'C:a'\n".to_string(), "absolute"));
        }
        for (text, why) in &refused {
            let e = parse(text, p).expect_err(text);
            assert_eq!(e.kind(), "parse", "{text}");
            assert!(e.to_string().contains(why), "{text}: {e}");
            assert!(e.to_string().contains("/cfg/workspaces.toml"), "{e}");
        }
    }

    #[test]
    fn round_trip_keeps_order_and_names() {
        let entries = vec![
            Entry {
                root: abs("/z/web").into(),
                name: None,
            },
            Entry {
                root: abs("/a/payments").into(),
                name: Some("Payments \"core\"".into()),
            },
        ];
        let text = render(&entries).expect("render");
        assert!(text.starts_with("# Projects"));
        assert_eq!(parse(&text, Path::new("x")).expect("parse"), entries);
    }

    #[test]
    fn add_refuses_non_repos_nested_folders_and_duplicates() {
        let (list, dir) = temp_list();
        let a = TempRepo::new(&[("src/lib.rs", "")]);
        let (e, new) = list.add(&a.root, None, false).expect("add a");
        assert!(new);
        assert_eq!(e.root, canonical(&a.root));

        // The same repository again, by its path.
        let again = list.add(&a.root, None, false).expect_err("dup");
        assert_eq!(again.kind(), "conflict");
        assert!(again.to_string().contains("already in the list"), "{again}");
        // ...which the app's "open the folder I started in" accepts.
        let (same, new) = list.add(&a.root, None, true).expect("ok if listed");
        assert!(!new);
        assert_eq!(same.id(), e.id());

        // A folder inside it.
        let nested = list
            .add(&a.root.join("src"), None, false)
            .expect_err("nested");
        assert!(
            nested.to_string().contains("inside the repository"),
            "{nested}"
        );

        // One of its linked worktrees (a run's, say) is the same repository.
        let wt = a.root.join(".git/kitsu/worktrees/r1");
        a.git()
            .worktree_add(&wt, "kitsu/run/r1", "HEAD")
            .expect("wt");
        let via_wt = list.add(&wt, None, false).expect_err("worktree");
        assert_eq!(via_wt.kind(), "conflict");
        assert!(
            via_wt.to_string().contains("one of its worktrees"),
            "{via_wt}"
        );

        // Not a repository at all, a file, nothing there.
        let plain = dir.join("plain");
        std::fs::create_dir_all(&plain).expect("mkdir");
        let e1 = list.add(&plain, None, false).expect_err("plain");
        assert!(e1.to_string().contains("isn't a git repository"), "{e1}");
        let file = dir.join("f.txt");
        std::fs::write(&file, "x").expect("write");
        assert!(
            list.add(&file, None, false)
                .expect_err("file")
                .to_string()
                .contains("not a folder")
        );
        assert_eq!(
            list.add(&dir.join("nope"), None, false)
                .expect_err("missing")
                .kind(),
            "not_found"
        );

        // None of the refusals wrote anything.
        assert_eq!(list.load().expect("load").len(), 1);

        // A second, distinct repository nested inside the first is its own.
        let b_root = a.root.join("vendor/b");
        std::fs::create_dir_all(&b_root).expect("mkdir");
        let g = Git::new(&b_root);
        g.run(["init", "--quiet", "-b", "main"]).expect("init");
        let (b, _) = list
            .add(&b_root, Some("  Bee  "), false)
            .expect("nested repo of its own");
        assert_eq!(b.display_name(), "Bee");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_rename_reorder_touch_only_the_list() {
        let (list, dir) = temp_list();
        let a = TempRepo::new(&[("a.txt", "a")]);
        let b = TempRepo::new(&[]);
        let c = TempRepo::new(&[]);
        let ids: Vec<String> = [&a, &b, &c]
            .iter()
            .map(|r| list.add(&r.root, None, false).expect("add").0.id())
            .collect();
        list.reorder(&ids[2], 0).expect("reorder");
        list.rename(&ids[0], Some("alpha")).expect("rename");
        let order: Vec<String> = list.load().expect("load").iter().map(Entry::id).collect();
        assert_eq!(order, [ids[2].clone(), ids[0].clone(), ids[1].clone()]);
        assert_eq!(
            list.find(&ids[0])
                .expect("find")
                .expect("some")
                .display_name(),
            "alpha"
        );
        list.rename(&ids[0], Some("")).expect("reset name");
        assert_eq!(list.find(&ids[0]).expect("find").expect("some").name, None);
        assert!(list.rename(&ids[0], Some(&"x".repeat(65))).is_err());

        let gone = list.remove(&ids[0]).expect("remove");
        assert_eq!(gone.root, canonical(&a.root));
        assert!(
            a.root.join("a.txt").exists(),
            "removing never deletes files"
        );
        assert_eq!(list.remove(&ids[0]).expect_err("twice").kind(), "not_found");
        assert_eq!(list.load().expect("load").len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_broken_file_is_reported_not_overwritten() {
        let (list, dir) = temp_list();
        std::fs::write(list.path(), "[[workspace]]\nroot = \"relative\"\n").expect("write");
        let a = TempRepo::new(&[]);
        assert_eq!(
            list.add(&a.root, None, false).expect_err("broken").kind(),
            "parse"
        );
        assert_eq!(
            std::fs::read_to_string(list.path()).expect("read"),
            "[[workspace]]\nroot = \"relative\"\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn overview_counts_come_from_the_status_snapshot() {
        let a = TempRepo::new(&[
            (".kitsu/kitsu.toml", ""),
            (".kitsu/tasks/one.md", "+++\ntitle = \"One\"\n+++\n"),
            (
                ".kitsu/tasks/two.md",
                "+++\ntitle = \"Two\"\nafter = [\"one\"]\n+++\n",
            ),
            (
                ".kitsu/questions/q.md",
                "+++\ntitle = \"Q?\"\nblocks = [\"three\"]\n+++\n",
            ),
            (".kitsu/tasks/three.md", "+++\ntitle = \"Three\"\n+++\n"),
        ]);
        let plain = TempRepo::new(&[]);
        let entries = vec![
            Entry {
                root: a.root.clone(),
                name: None,
            },
            Entry {
                root: plain.root.clone(),
                name: Some("plain".into()),
            },
            Entry {
                root: "/definitely/not/here".into(),
                name: None,
            },
        ];
        let o = overview(&entries);
        assert_eq!(o.len(), 3);
        assert_eq!(o[0].branch.as_deref(), Some("main"));
        assert!(o[0].initialized);
        assert_eq!(
            (o[0].needs_you, o[0].working, o[0].ready),
            (1, 0, 1),
            "{:?}",
            o[0]
        );
        assert!(o[0].error.is_none(), "{:?}", o[0].error);
        assert!(!o[1].initialized);
        assert_eq!(o[1].name, "plain");
        assert!(o[2].missing);
        assert!(o[2].error.as_deref().is_some_and(|e| e.contains("gone")));
    }

    #[test]
    fn cached_overview_follows_changes() {
        let a = TempRepo::new(&[
            (".kitsu/kitsu.toml", ""),
            (".kitsu/tasks/one.md", "+++\ntitle = \"One\"\n+++\n"),
        ]);
        let entries = vec![Entry {
            root: a.root.clone(),
            name: None,
        }];
        let cache = Cache::default();
        assert_eq!(cache.overview(&entries)[0].ready, 1);
        assert_eq!(cache.overview(&entries)[0].ready, 1);
        a.write(".kitsu/tasks/two.md", "+++\ntitle = \"Two\"\n+++\n");
        assert_eq!(cache.overview(&entries)[0].ready, 2, "a new task file");
        a.git()
            .run(["switch", "--quiet", "-c", "next"])
            .expect("switch");
        assert_eq!(cache.overview(&entries)[0].branch.as_deref(), Some("next"));
        let renamed = vec![Entry {
            root: a.root.clone(),
            name: Some("Renamed".into()),
        }];
        assert_eq!(cache.overview(&renamed)[0].name, "Renamed");
    }

    #[test]
    fn git_view_marks_run_worktrees_and_branches() {
        let a = TempRepo::new(&[("a.txt", "a")]);
        let ws = Workspace::discover(&a.root).expect("ws");
        let wt = ws.worktree_for("r1");
        a.git()
            .worktree_add(&wt, "kitsu/run/r1", "HEAD")
            .expect("run wt");
        let mine = a.root.with_extension("mine");
        a.git().worktree_add(&mine, "spike", "HEAD").expect("my wt");
        let v = git_view(&ws).expect("view");
        assert_eq!(v.branch.as_deref(), Some("main"));
        let kinds: Vec<String> = v
            .worktrees
            .iter()
            .map(|w| match &w.owner {
                Owner::Main => "main".to_string(),
                Owner::Run { run, .. } => format!("run:{run}"),
                Owner::Integration => "integration".into(),
                Owner::Yours => format!("yours:{}", w.branch.as_deref().unwrap_or("")),
            })
            .collect();
        assert_eq!(kinds.len(), 3);
        assert!(kinds.contains(&"main".to_string()), "{kinds:?}");
        assert!(kinds.contains(&"run:r1".to_string()), "{kinds:?}");
        assert!(kinds.contains(&"yours:spike".to_string()), "{kinds:?}");
        let run_branch = v
            .branches
            .iter()
            .find(|b| b.info.name == "kitsu/run/r1")
            .expect("run branch");
        assert_eq!(run_branch.run.as_deref(), Some("r1"));
        assert!(
            v.branches
                .iter()
                .find(|b| b.info.name == "spike")
                .expect("spike")
                .run
                .is_none()
        );
        let _ = std::fs::remove_dir_all(mine);
    }
}
