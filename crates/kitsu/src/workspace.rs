//! Where things live, and who owns what right now.
//!
//! ```text
//! <repo>/.kitsu/                  intent, tracked by git (see intent.rs)
//! <git-common-dir>/kitsu/         execution state, one per clone, never tracked
//!     state.db                    runs, evidence, events (store.rs)
//!     blobs/                      content-addressed logs and briefs
//!     worktrees/<run>/            one isolated checkout per run
//!     integrate/<id>/             scratch checkouts for building candidates
//!     owners/<instance>.lock      held for the life of each Kitsu process
//!     scratch/                    temporary index files
//! ```
//!
//! Putting state in the git common dir means every linked worktree of the
//! repo sees the same state, nothing shows up in `git status`, and deleting
//! the clone deletes the state.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::git::Git;
use crate::store::Store;
use crate::util::{sha256_hex, short_id};

#[derive(Debug, Clone)]
pub struct Workspace {
    /// The main worktree: where the human works and where intent is read.
    pub root: PathBuf,
    pub state: PathBuf,
}

impl Workspace {
    /// Find the repository containing `path`. Works from inside a run's
    /// worktree too; `root` is always the main worktree.
    pub fn discover(path: &Path) -> Result<Workspace> {
        let git = Git::new(path);
        let common = git.common_dir().map_err(|e| match e {
            Error::Git { .. } => {
                Error::NotFound(format!("{} is not inside a git repository", path.display()))
            }
            e => e,
        })?;
        // The first entry of `worktree list` is the main worktree.
        let root = git
            .worktree_paths()?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Invalid("git worktree list returned nothing".into()))?;
        let state = common.join("kitsu");
        Ok(Workspace { root, state })
    }

    pub fn git(&self) -> Git {
        Git::new(&self.root)
    }

    pub fn open_store(&self) -> Result<Store> {
        std::fs::create_dir_all(&self.state)
            .map_err(|e| Error::io(self.state.display().to_string(), e))?;
        Store::open(&self.state.join("state.db"))
    }

    pub fn worktree_for(&self, run: &str) -> PathBuf {
        self.state.join("worktrees").join(run)
    }

    pub fn integration_dir(&self, id: &str) -> PathBuf {
        self.state.join("integrate").join(id)
    }

    pub fn scratch(&self) -> PathBuf {
        self.state.join("scratch")
    }

    /// A path that is never created. Passed as `core.hooksPath` so Kitsu's
    /// own commits don't run repository hooks.
    pub fn no_hooks(&self) -> PathBuf {
        self.state.join("no-hooks")
    }

    pub fn blobs(&self) -> Blobs {
        Blobs {
            dir: self.state.join("blobs"),
        }
    }

    /// If `path` is inside one of this workspace's run worktrees, which run?
    pub fn run_for_path(&self, path: &Path) -> Option<String> {
        let base = self.state.join("worktrees");
        let rel = path.strip_prefix(&base).ok()?;
        rel.components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
    }

    pub fn is_trusted(&self) -> Result<bool> {
        let key = self.trust_key();
        Ok(read_lines(&config_dir().join("trusted"))?
            .iter()
            .any(|l| l == &key))
    }

    pub fn trust(&self) -> Result<()> {
        if self.is_trusted()? {
            return Ok(());
        }
        let dir = config_dir();
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(dir.display().to_string(), e))?;
        let path = dir.join("trusted");
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| Error::io(path.display().to_string(), e))?;
        writeln!(f, "{}", self.trust_key()).map_err(|e| Error::io(path.display().to_string(), e))
    }

    fn trust_key(&self) -> String {
        std::fs::canonicalize(&self.root)
            .unwrap_or_else(|_| self.root.clone())
            .display()
            .to_string()
    }
}

/// `$KITSU_CONFIG_DIR`, else the platform config dir + `/kitsu`.
pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("KITSU_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.unwrap_or_else(|| PathBuf::from(".")).join("kitsu")
}

fn read_lines(path: &Path) -> Result<Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(Error::io(path.display().to_string(), e)),
    }
}

/// Custody token for one Kitsu process. The lock file is held exclusively
/// for as long as this value lives. The OS drops the lock when the process
/// dies for any reason, including `kill -9`, which is what lets another
/// process decide "the owner of this run is gone" without heartbeats.
pub struct Instance {
    pub id: String,
    _lock: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Alive,
    Dead,
}

impl Instance {
    pub fn acquire(ws: &Workspace) -> Result<Instance> {
        let dir = ws.state.join("owners");
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(dir.display().to_string(), e))?;
        for _ in 0..8 {
            let id = short_id('p');
            let path = dir.join(format!("{id}.lock"));
            let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::io(path.display().to_string(), e)),
            };
            file.try_lock().map_err(|e| {
                Error::Invalid(format!(
                    "could not lock fresh owner file {}: {e}",
                    path.display()
                ))
            })?;
            return Ok(Instance { id, _lock: file });
        }
        Err(Error::Invalid("could not allocate an owner id".into()))
    }

    /// Is the process holding `id` still running?
    pub fn liveness(ws: &Workspace, id: &str) -> Result<Liveness> {
        let path = ws.state.join("owners").join(format!("{id}.lock"));
        let file = match OpenOptions::new().write(true).open(&path) {
            Ok(f) => f,
            // Removed by an earlier recovery, or never created.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Liveness::Dead),
            Err(e) => return Err(Error::io(path.display().to_string(), e)),
        };
        match file.try_lock() {
            Ok(()) => {
                // We hold it, so nobody else does. Remove it while holding the
                // lock so a racing check sees NotFound, which also means dead.
                let _ = std::fs::remove_file(&path);
                Ok(Liveness::Dead)
            }
            Err(TryLockError::WouldBlock) => Ok(Liveness::Alive),
            Err(TryLockError::Error(e)) => Err(Error::io(format!("locking {}", path.display()), e)),
        }
    }
}

/// Write-once, content-addressed files. Used for check logs and for the
/// exact brief each run received.
#[derive(Debug, Clone)]
pub struct Blobs {
    dir: PathBuf,
}

impl Blobs {
    pub fn put(&self, bytes: &[u8]) -> Result<String> {
        let id = sha256_hex(bytes);
        let path = self.path(&id);
        if path.exists() {
            return Ok(id);
        }
        let parent = path
            .parent()
            .ok_or_else(|| Error::Invalid("blob path".into()))?;
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent.display().to_string(), e))?;
        let tmp = parent.join(format!(".{id}.{}", short_id('w')));
        std::fs::write(&tmp, bytes).map_err(|e| Error::io(tmp.display().to_string(), e))?;
        std::fs::rename(&tmp, &path).map_err(|e| Error::io(path.display().to_string(), e))?;
        Ok(id)
    }

    pub fn get(&self, id: &str) -> Result<Vec<u8>> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid(format!("bad blob id {id}")));
        }
        let path = self.path(id);
        std::fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                Error::NotFound(format!("blob {id} (pruned or never written)"))
            }
            _ => Error::io(path.display().to_string(), e),
        })
    }

    pub fn path(&self, id: &str) -> PathBuf {
        self.dir.join(&id[..2.min(id.len())]).join(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::TempRepo;

    #[test]
    fn liveness_follows_the_lock() {
        let repo = TempRepo::new(&[]);
        let ws = Workspace::discover(&repo.root).expect("ws");
        let me = Instance::acquire(&ws).expect("acquire");
        assert_eq!(
            Instance::liveness(&ws, &me.id).expect("live"),
            Liveness::Alive
        );
        let id = me.id.clone();
        drop(me);
        assert_eq!(Instance::liveness(&ws, &id).expect("dead"), Liveness::Dead);
        // Second check after cleanup is still Dead, not an error.
        assert_eq!(
            Instance::liveness(&ws, &id).expect("dead again"),
            Liveness::Dead
        );
    }

    #[test]
    fn discover_from_a_linked_worktree_finds_the_main_one() {
        let repo = TempRepo::new(&[("a", "a")]);
        let ws = Workspace::discover(&repo.root).expect("ws");
        let wt = ws.worktree_for("r1");
        ws.git()
            .worktree_add(&wt, "kitsu/run/r1", "HEAD")
            .expect("add");
        let from_wt = Workspace::discover(&wt).expect("ws from wt");
        assert_eq!(
            std::fs::canonicalize(&from_wt.root).ok(),
            std::fs::canonicalize(&repo.root).ok()
        );
        assert_eq!(from_wt.run_for_path(&wt).as_deref(), Some("r1"));
        // And the main worktree's status does not see it.
        assert!(repo.git().is_clean().expect("clean"));
    }

    #[test]
    fn blobs_round_trip() {
        let repo = TempRepo::new(&[]);
        let ws = Workspace::discover(&repo.root).expect("ws");
        let b = ws.blobs();
        let id = b.put(b"hello").expect("put");
        assert_eq!(b.put(b"hello").expect("again"), id);
        assert_eq!(b.get(&id).expect("get"), b"hello");
        assert_eq!(
            b.get(&"0".repeat(64)).expect_err("missing").kind(),
            "not_found"
        );
    }
}
