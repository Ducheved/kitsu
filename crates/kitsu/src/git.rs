//! Git, through the git binary.
//!
//! Kitsu shells out instead of linking a git library. The user's git has
//! their config, their credential helpers and exactly the semantics they
//! know, and every operation here is a handful of milliseconds off the
//! interactive path. The cost is a process spawn per call; see
//! docs/design.md for where that stops being acceptable.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::error::{Error, Result};

/// Environment variables that would silently redirect git to another
/// repository or index. Kitsu is sometimes launched from inside git hooks or
/// editors that set them.
const LEAKY_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
];

pub const NULL_OID: &str = "0000000000000000000000000000000000000000";

#[derive(Debug, Clone)]
pub struct Git {
    /// Working directory the command runs in.
    dir: PathBuf,
}

/// One line of `git diff --numstat`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileStat {
    pub path: String,
    /// `None` for binary files.
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Git { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.dir);
        for var in LEAKY_ENV {
            cmd.env_remove(var);
        }
        // Never block on a credential or editor prompt.
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("GIT_EDITOR", "true");
        cmd.stdin(Stdio::null());
        cmd.args(args);
        cmd
    }

    fn output_of(&self, mut cmd: Command, label: String) -> Result<Output> {
        let out = cmd
            .output()
            .map_err(|e| Error::io(format!("spawning git {label}"), e))?;
        if out.status.success() {
            Ok(out)
        } else {
            Err(Error::Git {
                args: label,
                code: out.status.code(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            })
        }
    }

    pub fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S> + Clone,
        S: AsRef<OsStr>,
    {
        let label = label(args.clone());
        let out = self.output_of(self.command(args), label)?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn run_bytes<I, S>(&self, args: I) -> Result<Vec<u8>>
    where
        I: IntoIterator<Item = S> + Clone,
        S: AsRef<OsStr>,
    {
        let label = label(args.clone());
        Ok(self.output_of(self.command(args), label)?.stdout)
    }

    /// Like `run`, but reports a non-zero exit as `Ok(false)` instead of an
    /// error. For git commands that answer yes/no through the exit code.
    fn test<I, S>(&self, args: I) -> Result<bool>
    where
        I: IntoIterator<Item = S> + Clone,
        S: AsRef<OsStr>,
    {
        let label = label(args.clone());
        let out = self
            .command(args)
            .output()
            .map_err(|e| Error::io(format!("spawning git {label}"), e))?;
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            code => Err(Error::Git {
                args: label,
                code,
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }),
        }
    }

    pub fn toplevel(&self) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.run(["rev-parse", "--show-toplevel"])?.trim(),
        ))
    }

    /// The directory shared by all worktrees of this repository.
    pub fn common_dir(&self) -> Result<PathBuf> {
        let raw = self.run(["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
        Ok(PathBuf::from(raw.trim()))
    }

    pub fn head(&self) -> Result<Option<String>> {
        match self.run(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]) {
            Ok(s) => Ok(Some(s.trim().to_string())),
            Err(Error::Git { code: Some(1), .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn rev(&self, rev: &str) -> Result<String> {
        Ok(self
            .run([
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{rev}^{{commit}}"),
            ])?
            .trim()
            .to_string())
    }

    /// `None` on a detached HEAD.
    pub fn current_branch(&self) -> Result<Option<String>> {
        let out = self.run(["symbolic-ref", "--quiet", "--short", "HEAD"]);
        match out {
            Ok(s) => Ok(Some(s.trim().to_string())),
            Err(Error::Git { code: Some(1), .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn branch_head(&self, branch: &str) -> Result<Option<String>> {
        let r = format!("refs/heads/{branch}");
        match self.run(["rev-parse", "--verify", "--quiet", &r]) {
            Ok(s) => Ok(Some(s.trim().to_string())),
            Err(Error::Git { code: Some(1), .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn tree_of(&self, commit: &str) -> Result<String> {
        Ok(self
            .run(["rev-parse", "--verify", &format!("{commit}^{{tree}}")])?
            .trim()
            .to_string())
    }

    /// Tree id of the working directory as it is on disk right now, including
    /// untracked files that are not ignored. Uses a scratch copy of the index
    /// so the user's staging area is never touched. Cost is one stat per
    /// tracked file plus hashing of whatever changed.
    pub fn worktree_tree(&self, scratch_dir: &Path) -> Result<String> {
        let git_dir = PathBuf::from(
            self.run(["rev-parse", "--path-format=absolute", "--git-dir"])?
                .trim(),
        );
        std::fs::create_dir_all(scratch_dir)
            .map_err(|e| Error::io(scratch_dir.display().to_string(), e))?;
        let tmp = scratch_dir.join(format!(
            "index.{}.{}",
            std::process::id(),
            crate::util::short_id('i')
        ));
        let real = git_dir.join("index");
        if real.exists() {
            std::fs::copy(&real, &tmp)
                .map_err(|e| Error::io(format!("copying {}", real.display()), e))?;
        }
        let result = (|| {
            self.unhide_in(&tmp)?;
            // Stat shortcuts can skip a modified file; this hash is what
            // evidence gets bound to, so don't take them.
            let mut add = self.command([
                "-c",
                "core.ignoreStat=false",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.untrackedCache=false",
                "add",
                "-A",
                "--",
                ".",
            ]);
            add.env("GIT_INDEX_FILE", &tmp);
            self.output_of(add, "add -A (scratch index)".into())?;
            let mut write = self.command(["write-tree"]);
            write.env("GIT_INDEX_FILE", &tmp);
            let out = self.output_of(write, "write-tree (scratch index)".into())?;
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        })();
        let _ = std::fs::remove_file(&tmp);
        result
    }

    /// In a scratch index, clear the flags that make `git add` ignore a
    /// modified file: assume-unchanged, and skip-worktree for files that are
    /// actually on disk. A skip-worktree file that isn't there (a sparse
    /// checkout) keeps its flag, so its absence isn't read as a deletion.
    fn unhide_in(&self, index: &Path) -> Result<()> {
        let mut ls = self.command(["ls-files", "-v", "-z"]);
        ls.env("GIT_INDEX_FILE", index);
        let out = self.output_of(ls, "ls-files -v (scratch index)".into())?;
        let mut assumed = String::new();
        let mut skipped = String::new();
        for rec in split_z(&out.stdout) {
            let Some((tag, path)) = rec.split_once(' ') else {
                continue;
            };
            let tag = tag.chars().next().unwrap_or('H');
            if tag.is_ascii_lowercase() {
                assumed.push_str(path);
                assumed.push('\0');
            }
            if tag.eq_ignore_ascii_case(&'s') && self.dir.join(path).exists() {
                skipped.push_str(path);
                skipped.push('\0');
            }
        }
        for (flag, paths) in [
            ("--no-assume-unchanged", assumed),
            ("--no-skip-worktree", skipped),
        ] {
            if paths.is_empty() {
                continue;
            }
            let mut cmd = self.command(["update-index", flag, "-z", "--stdin"]);
            cmd.env("GIT_INDEX_FILE", index)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            let mut child = cmd
                .spawn()
                .map_err(|e| Error::io("spawning git update-index", e))?;
            {
                use std::io::Write;
                let mut stdin = child
                    .stdin
                    .take()
                    .ok_or_else(|| Error::Invalid("update-index stdin".into()))?;
                stdin
                    .write_all(paths.as_bytes())
                    .map_err(|e| Error::io("writing to git update-index", e))?;
            }
            let out = child
                .wait_with_output()
                .map_err(|e| Error::io("git update-index", e))?;
            if !out.status.success() {
                return Err(Error::Git {
                    args: format!("update-index {flag}"),
                    code: out.status.code(),
                    stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                });
            }
        }
        Ok(())
    }

    /// Paths that differ between two trees (or commits).
    pub fn changed_paths(&self, a: &str, b: &str) -> Result<Vec<String>> {
        if a == b {
            return Ok(Vec::new());
        }
        let out = self.run_bytes(["diff-tree", "-r", "-z", "--name-only", "--no-renames", a, b])?;
        Ok(split_z(&out))
    }

    pub fn numstat(&self, a: &str, b: &str) -> Result<Vec<FileStat>> {
        let out = self.run_bytes(["diff", "--numstat", "-z", "--no-renames", a, b])?;
        let mut stats = Vec::new();
        for rec in split_z(&out) {
            let mut parts = rec.splitn(3, '\t');
            let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            stats.push(FileStat {
                path: path.to_string(),
                added: add.parse().ok(),
                removed: del.parse().ok(),
            });
        }
        Ok(stats)
    }

    pub fn diff(&self, a: &str, b: &str, paths: &[&str]) -> Result<String> {
        let mut args: Vec<&str> = vec!["diff", "--no-color", "--no-ext-diff", a, b, "--"];
        args.extend_from_slice(paths);
        self.run(args)
    }

    pub fn merge_base(&self, a: &str, b: &str) -> Result<String> {
        Ok(self.run(["merge-base", a, b])?.trim().to_string())
    }

    /// Is `path` tracked at HEAD and identical on disk?
    pub fn is_clean_path(&self, path: &str) -> Result<bool> {
        let tracked = self
            .test(["ls-files", "--error-unmatch", "--", path])
            .unwrap_or(false);
        if !tracked {
            return Ok(false);
        }
        Ok(self
            .run(["status", "--porcelain=v1", "--", path])?
            .trim()
            .is_empty())
    }

    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        self.test(["merge-base", "--is-ancestor", ancestor, descendant])
    }

    /// Compare-and-swap a ref. Fails with `Conflict` if the ref is not at
    /// `old` anymore. This is the fencing primitive for integration.
    pub fn update_ref_cas(&self, refname: &str, new: &str, old: &str) -> Result<()> {
        match self.run(["update-ref", "-m", "kitsu accept", refname, new, old]) {
            Ok(_) => Ok(()),
            Err(Error::Git { stderr, .. })
                if stderr.contains("but expected") || stderr.contains("cannot lock ref") =>
            {
                Err(Error::Conflict(format!(
                    "{refname} moved: {}",
                    stderr.trim()
                )))
            }
            Err(e) => Err(e),
        }
    }

    pub fn is_clean(&self) -> Result<bool> {
        Ok(self
            .run(["status", "--porcelain=v1", "--untracked-files=normal"])?
            .trim()
            .is_empty())
    }

    pub fn worktree_add(&self, path: &Path, branch: &str, base: &str) -> Result<()> {
        let p = path.to_string_lossy();
        self.run(["worktree", "add", "--quiet", "-b", branch, &p, base])
            .map(|_| ())
    }

    pub fn worktree_add_detached(&self, path: &Path, base: &str) -> Result<()> {
        let p = path.to_string_lossy();
        self.run(["worktree", "add", "--quiet", "--detach", &p, base])
            .map(|_| ())
    }

    pub fn worktree_remove(&self, path: &Path) -> Result<()> {
        let p = path.to_string_lossy();
        self.run(["worktree", "remove", "--force", &p]).map(|_| ())
    }

    pub fn worktree_paths(&self) -> Result<Vec<PathBuf>> {
        let out = self.run(["worktree", "list", "--porcelain", "-z"])?;
        Ok(out
            .split('\0')
            .filter_map(|l| l.strip_prefix("worktree "))
            .map(PathBuf::from)
            .collect())
    }

    /// Every worktree git knows about, main one first. Callers that may race
    /// a `worktree add` should hold `Workspace::lock_worktrees` around this.
    pub fn worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let out = self.run(["worktree", "list", "--porcelain", "-z"])?;
        Ok(parse_worktrees(&out))
    }

    /// Local branches with their upstream and how far apart they are. One
    /// `for-each-ref`; git computes ahead/behind itself.
    pub fn branches(&self) -> Result<Vec<BranchInfo>> {
        let out = self.run([
            "for-each-ref",
            "--format=%(HEAD)%00%(refname:short)%00%(objectname:short)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(committerdate:unix)%00%(contents:subject)",
            "refs/heads",
        ])?;
        Ok(out.lines().filter_map(parse_branch).collect())
    }

    pub fn prune_worktrees(&self) -> Result<()> {
        self.run(["worktree", "prune"]).map(|_| ())
    }

    pub fn delete_branch(&self, branch: &str) -> Result<()> {
        self.run(["branch", "-D", "--quiet", branch]).map(|_| ())
    }

    /// Commit everything in this worktree as a machine snapshot. Hooks and
    /// signing are off and the identity is fixed: these commits live on
    /// `kitsu/run/*` branches and are never what lands on the user's branch.
    /// Returns the new HEAD (or the old one if there was nothing to commit).
    pub fn snapshot(&self, message: &str, no_hooks_dir: &Path) -> Result<String> {
        self.run(["add", "-A", "--", "."])?;
        let staged = !self.test(["diff", "--cached", "--quiet"])?;
        if staged {
            let hooks = format!("core.hooksPath={}", no_hooks_dir.display());
            self.run([
                "-c",
                &hooks,
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=kitsu",
                "-c",
                "user.email=kitsu@localhost",
                "commit",
                "--quiet",
                "--no-verify",
                "-m",
                message,
            ])?;
        }
        self.head()?
            .ok_or_else(|| Error::Invalid("snapshot of a repository with no commits".into()))
    }

    /// Commit the staged state with the user's own identity (they are the one
    /// accepting). Hooks are off; Kitsu already ran the checks it was told to.
    pub fn commit_as_user(&self, message: &str, no_hooks_dir: &Path) -> Result<String> {
        let hooks = format!("core.hooksPath={}", no_hooks_dir.display());
        self.run([
            "-c",
            &hooks,
            "commit",
            "--quiet",
            "--no-verify",
            "--allow-empty",
            "-m",
            message,
        ])?;
        self.head()?
            .ok_or_else(|| Error::Invalid("commit produced no HEAD".into()))
    }

    /// Files under `prefix` at `rev`, with contents. Used to read the rules a
    /// candidate is judged by from the target branch rather than from the
    /// candidate itself.
    pub fn files_at(&self, rev: &str, prefix: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let blobs = self.tree_blobs(rev, prefix)?;
        let contents = self.read_blobs(blobs.iter().map(|b| b.oid.as_str()))?;
        Ok(blobs.into_iter().map(|b| b.path).zip(contents).collect())
    }

    /// Every blob under `prefix` at `rev` (empty prefix: the whole tree),
    /// with its size. Submodules and symlinks are skipped.
    pub fn tree_blobs(&self, rev: &str, prefix: &str) -> Result<Vec<TreeBlob>> {
        let mut args = vec!["ls-tree", "-r", "-z", "-l", "--full-tree", rev];
        if !prefix.is_empty() {
            args.extend(["--", prefix]);
        }
        let listing = self.run_bytes(args)?;
        let mut blobs = Vec::new();
        for rec in split_z(&listing) {
            // "<mode> <type> <oid> <size padded>\t<path>"
            let Some((meta, path)) = rec.split_once('\t') else {
                continue;
            };
            let mut it = meta.split_whitespace();
            let (mode, kind, oid, size) = (it.next(), it.next(), it.next(), it.next());
            if kind == Some("blob")
                && mode != Some("120000")
                && let (Some(oid), Some(size)) = (oid, size.and_then(|s| s.parse().ok()))
            {
                blobs.push(TreeBlob {
                    path: path.to_string(),
                    oid: oid.to_string(),
                    size,
                });
            }
        }
        Ok(blobs)
    }

    /// Contents of the given blobs, in order, through one `cat-file --batch`.
    /// Ids are written from another thread: writing them all first and then
    /// reading deadlocks once both pipe buffers are full.
    pub fn read_blobs<'a>(&self, oids: impl IntoIterator<Item = &'a str>) -> Result<Vec<Vec<u8>>> {
        let mut input = String::new();
        let mut want = Vec::new();
        for oid in oids {
            input.push_str(oid);
            input.push('\n');
            want.push(oid.to_string());
        }
        if want.is_empty() {
            return Ok(Vec::new());
        }
        let mut cmd = self.command(["cat-file", "--batch"]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::io("spawning git cat-file", e))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Invalid("cat-file stdin".into()))?;
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            stdin.write_all(input.as_bytes())
        });
        let out = child
            .wait_with_output()
            .map_err(|e| Error::io("git cat-file", e))?;
        writer
            .join()
            .map_err(|_| Error::Invalid("cat-file writer panicked".into()))?
            .map_err(|e| Error::io("writing to git cat-file", e))?;
        if !out.status.success() {
            return Err(Error::Git {
                args: "cat-file --batch".into(),
                code: out.status.code(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }
        parse_batch(&out.stdout, &want)
    }

    /// Squash-merge `commit` into the current (clean) worktree and leave the
    /// result staged. Returns the conflicting paths on conflict, after
    /// resetting the worktree.
    pub fn merge_squash(&self, commit: &str) -> Result<Result<(), Vec<String>>> {
        let out = self
            .command([
                "-c",
                "merge.conflictstyle=merge",
                "merge",
                "--squash",
                "--no-commit",
                commit,
            ])
            .output()
            .map_err(|e| Error::io("spawning git merge --squash", e))?;
        if out.status.success() {
            return Ok(Ok(()));
        }
        let conflicts =
            split_z(&self.run_bytes(["diff", "--name-only", "-z", "--diff-filter=U"])?);
        self.run(["reset", "--hard", "--quiet", "HEAD"])?;
        if conflicts.is_empty() {
            return Err(Error::Git {
                args: format!("merge --squash {commit}"),
                code: out.status.code(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }
        Ok(Err(conflicts))
    }

    pub fn merge_ff_only(&self, commit: &str) -> Result<()> {
        match self.run(["merge", "--ff-only", "--quiet", commit]) {
            Ok(_) => Ok(()),
            Err(Error::Git { stderr, .. }) => Err(Error::Conflict(format!(
                "could not fast-forward: {}",
                stderr.trim()
            ))),
            Err(e) => Err(e),
        }
    }

    pub fn add_path(&self, path: &str) -> Result<()> {
        self.run(["add", "--", path]).map(|_| ())
    }

    pub fn show_file(&self, rev: &str, path: &str) -> Result<Option<Vec<u8>>> {
        match self.run_bytes(["show", &format!("{rev}:{path}")]) {
            Ok(b) => Ok(Some(b)),
            Err(Error::Git {
                code: Some(128), ..
            }) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn label<I, S>(args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_z(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

#[derive(Debug, Clone)]
pub struct TreeBlob {
    pub path: String,
    pub oid: String,
    pub size: u64,
}

fn parse_batch(mut out: &[u8], oids: &[String]) -> Result<Vec<Vec<u8>>> {
    let mut files = Vec::with_capacity(oids.len());
    for oid in oids {
        let nl = out
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| Error::Protocol("truncated cat-file header".into()))?;
        let header = String::from_utf8_lossy(&out[..nl]).into_owned();
        let mut parts = header.split(' ');
        let (got, _kind, size) = (parts.next(), parts.next(), parts.next());
        if got != Some(oid.as_str()) {
            return Err(Error::Invalid(format!(
                "cat-file answered {header:?} for {oid}"
            )));
        }
        let size: usize = size
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::Invalid(format!("bad cat-file header {header:?}")))?;
        let start = nl + 1;
        let end = start + size;
        if out.len() < end + 1 {
            return Err(Error::Invalid("truncated cat-file body".into()));
        }
        files.push(out[start..end].to_vec());
        out = &out[end + 1..];
    }
    Ok(files)
}

/// One entry of `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub head: Option<String>,
    /// Short branch name; `None` when detached or bare.
    pub branch: Option<String>,
    pub bare: bool,
    pub locked: bool,
    /// Git thinks the directory is gone.
    pub prunable: bool,
}

fn parse_worktrees(out: &str) -> Vec<WorktreeInfo> {
    let mut all = Vec::new();
    let mut cur: Option<WorktreeInfo> = None;
    // Records are NUL-terminated lines; an empty line ends a worktree.
    for line in out.split('\0') {
        if let Some(p) = line.strip_prefix("worktree ") {
            all.extend(cur.take());
            cur = Some(WorktreeInfo {
                path: PathBuf::from(p),
                head: None,
                branch: None,
                bare: false,
                locked: false,
                prunable: false,
            });
            continue;
        }
        let Some(w) = cur.as_mut() else { continue };
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "HEAD" => w.head = Some(value.to_string()),
            "branch" => {
                w.branch = Some(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_string(),
                );
            }
            "bare" => w.bare = true,
            "locked" => w.locked = true,
            "prunable" => w.prunable = true,
            _ => {}
        }
    }
    all.extend(cur);
    all
}

/// One local branch, from `for-each-ref`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub head: String,
    pub current: bool,
    pub upstream: Option<String>,
    /// Commits here that the upstream doesn't have. `None` without an
    /// upstream, or when the upstream is gone.
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    /// The upstream branch was deleted on the remote.
    pub gone: bool,
    /// Committer date of the tip, Unix seconds.
    pub date: i64,
    pub subject: String,
}

fn parse_branch(line: &str) -> Option<BranchInfo> {
    let f: Vec<&str> = line.splitn(7, '\0').collect();
    let [head_mark, name, oid, upstream, track, date, subject] = f.as_slice() else {
        return None;
    };
    let upstream = (!upstream.is_empty()).then(|| upstream.to_string());
    let gone = *track == "gone";
    // "ahead 2, behind 1" / "ahead 2" / "behind 1" / "" (in sync).
    let mut ahead = upstream.as_ref().filter(|_| !gone).map(|_| 0);
    let mut behind = ahead;
    for part in track.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().ok();
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().ok();
        }
    }
    Some(BranchInfo {
        name: name.to_string(),
        head: oid.to_string(),
        current: *head_mark == "*",
        upstream,
        ahead,
        behind,
        gone,
        date: date.parse().unwrap_or(0),
        subject: subject.to_string(),
    })
}

#[cfg(test)]
pub(crate) mod testing {
    use super::Git;
    use std::path::PathBuf;

    /// A throwaway repository with one commit. Deleted on drop.
    pub struct TempRepo {
        pub root: PathBuf,
    }

    impl TempRepo {
        pub fn new(files: &[(&str, &str)]) -> TempRepo {
            let root =
                std::env::temp_dir().join(format!("kitsu-test-{}", crate::util::short_id('t')));
            std::fs::create_dir_all(&root).expect("temp dir");
            let git = Git::new(&root);
            git.run(["init", "--quiet", "-b", "main"])
                .expect("git init");
            git.run(["config", "user.name", "Test"]).expect("config");
            git.run(["config", "user.email", "test@example.com"])
                .expect("config");
            git.run(["config", "commit.gpgsign", "false"])
                .expect("config");
            let repo = TempRepo { root };
            for (p, c) in files {
                repo.write(p, c);
            }
            git.run(["add", "-A"]).expect("add");
            git.run(["commit", "--quiet", "--allow-empty", "-m", "init"])
                .expect("commit");
            repo
        }

        pub fn git(&self) -> Git {
            Git::new(&self.root)
        }

        pub fn write(&self, path: &str, contents: &str) {
            let p = self.root.join(path);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(p, contents).expect("write");
        }

        pub fn commit_all(&self, msg: &str) -> String {
            let git = self.git();
            git.run(["add", "-A"]).expect("add");
            git.run(["commit", "--quiet", "-m", msg]).expect("commit");
            git.head().expect("head").expect("some head")
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::TempRepo;

    #[test]
    fn worktree_tree_sees_changes_git_was_told_to_ignore() {
        // Evidence is bound to this hash. If a flag in the user's index or
        // config makes `git add` skip a modified file, a check that ran on
        // the modified file would be recorded against the old content.
        let repo = TempRepo::new(&[("a.txt", "1\n"), ("b.txt", "1\n"), ("c.txt", "1\n")]);
        let git = repo.git();
        let scratch = repo.root.join(".git/kitsu/scratch");
        let clean = git.worktree_tree(&scratch).expect("tree");
        git.run(["update-index", "--assume-unchanged", "a.txt"])
            .expect("assume");
        git.run(["update-index", "--skip-worktree", "b.txt"])
            .expect("skip");
        git.run(["config", "core.ignoreStat", "true"])
            .expect("ignorestat");
        repo.write("a.txt", "2\n");
        repo.write("b.txt", "2\n");
        repo.write("c.txt", "2\n");
        let dirty = git.worktree_tree(&scratch).expect("tree");
        let changed = git.changed_paths(&clean, &dirty).expect("diff");
        assert_eq!(
            changed,
            ["a.txt", "b.txt", "c.txt"],
            "every modified file is in the hash"
        );
        let flags = git
            .run(["ls-files", "-v", "a.txt", "b.txt"])
            .expect("ls-files");
        assert_eq!(
            flags, "h a.txt\nS b.txt\n",
            "the user's own index keeps its flags"
        );
        // A sparse checkout's skip-worktree files are simply absent: that is
        // not a deletion.
        std::fs::remove_file(repo.root.join("b.txt")).expect("rm");
        let sparse = git.worktree_tree(&scratch).expect("tree");
        assert_eq!(
            git.changed_paths(&clean, &sparse).expect("diff"),
            ["a.txt", "c.txt"]
        );
    }

    #[test]
    fn reading_many_blobs_does_not_deadlock_on_full_pipes() {
        // ~3000 ids (> 64 KiB of stdin) and > 64 KiB of output: the old
        // write-everything-then-read code hung here.
        let files: Vec<(String, String)> = (0..3000)
            .map(|i| {
                (
                    format!("f/{i}.txt"),
                    format!("file number {i} {}\n", "x".repeat(40)),
                )
            })
            .collect();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();
        let repo = TempRepo::new(&refs);
        let git = repo.git();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let root = repo.root.clone();
        std::thread::spawn(move || {
            let _ = tx.send(super::Git::new(&root).files_at("HEAD", "f"));
        });
        let got = rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("files_at finished instead of deadlocking")
            .expect("files");
        assert_eq!(got.len(), 3000);
        let one = got.iter().find(|(p, _)| p == "f/7.txt").expect("f/7");
        assert!(String::from_utf8_lossy(&one.1).starts_with("file number 7 "));
        assert_eq!(git.tree_blobs("HEAD", "").expect("blobs").len(), 3000);
    }

    #[test]
    fn worktree_tree_sees_untracked_and_leaves_index_alone() {
        let repo = TempRepo::new(&[("a.txt", "a\n")]);
        let git = repo.git();
        let scratch = repo.root.join(".git/kitsu-scratch");
        let clean = git.worktree_tree(&scratch).expect("tree");
        assert_eq!(clean, git.tree_of("HEAD").expect("head tree"));
        repo.write("b.txt", "b\n");
        let dirty = git.worktree_tree(&scratch).expect("tree");
        assert_ne!(clean, dirty);
        assert_eq!(
            git.changed_paths(&clean, &dirty).expect("diff"),
            vec!["b.txt".to_string()]
        );
        // The user's index still does not know about b.txt.
        assert!(
            git.run(["status", "--porcelain"])
                .expect("status")
                .contains("?? b.txt")
        );
    }

    #[test]
    fn cas_refuses_a_moved_ref() {
        let repo = TempRepo::new(&[("a.txt", "a\n")]);
        let git = repo.git();
        let first = git.head().expect("head").expect("some");
        repo.write("a.txt", "b\n");
        let second = repo.commit_all("second");
        let err = git
            .update_ref_cas("refs/heads/main", &first, &first)
            .expect_err("must refuse");
        assert_eq!(err.kind(), "conflict");
        git.update_ref_cas("refs/heads/main", &first, &second)
            .expect("cas ok");
        assert_eq!(git.head().expect("head").as_deref(), Some(first.as_str()));
    }

    #[test]
    fn files_at_reads_committed_state_only() {
        let repo = TempRepo::new(&[
            (".kitsu/kitsu.toml", "x = 1\n"),
            (".kitsu/tasks/a.md", "committed"),
        ]);
        repo.write(".kitsu/tasks/a.md", "dirty");
        let files = repo.git().files_at("HEAD", ".kitsu").expect("files");
        let a = files
            .iter()
            .find(|(p, _)| p == ".kitsu/tasks/a.md")
            .expect("a.md");
        assert_eq!(a.1, b"committed");
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn squash_merge_reports_conflicts() {
        let repo = TempRepo::new(&[("a.txt", "base\n")]);
        let git = repo.git();
        git.run(["checkout", "--quiet", "-b", "side"])
            .expect("branch");
        repo.write("a.txt", "side\n");
        let side = repo.commit_all("side");
        git.run(["checkout", "--quiet", "main"]).expect("main");
        repo.write("a.txt", "main\n");
        repo.commit_all("main");
        let res = git.merge_squash(&side).expect("merge ran");
        assert_eq!(res, Err(vec!["a.txt".to_string()]));
        assert!(git.is_clean().expect("status"));
    }

    #[test]
    fn branches_report_upstream_distance() {
        let repo = TempRepo::new(&[("a.txt", "1\n")]);
        let git = repo.git();
        git.run(["branch", "feature"]).expect("branch");
        git.run(["branch", "--set-upstream-to=main", "feature"])
            .expect("upstream");
        git.run(["switch", "--quiet", "feature"]).expect("switch");
        repo.write("a.txt", "2\n");
        repo.commit_all("ahead one");
        git.run(["switch", "--quiet", "main"]).expect("switch");
        repo.write("b.txt", "1\n");
        repo.commit_all("main moves");
        repo.write("b.txt", "2\n");
        repo.commit_all("main moves again");

        let all = git.branches().expect("branches");
        let names: Vec<&str> = all.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["feature", "main"]);
        let feature = &all[0];
        assert!(!feature.current);
        assert_eq!(feature.upstream.as_deref(), Some("main"));
        assert_eq!((feature.ahead, feature.behind), (Some(1), Some(2)));
        assert_eq!(feature.subject, "ahead one");
        let main = &all[1];
        assert!(main.current);
        assert_eq!(
            (main.upstream.as_ref(), main.ahead, main.behind),
            (None, None, None)
        );
    }

    #[test]
    fn worktrees_parse_porcelain() {
        let out = "worktree /r\0HEAD aaa\0branch refs/heads/main\0\0\
                   worktree /r/.git/kitsu/worktrees/r1\0HEAD bbb\0detached\0locked\0\0\
                   worktree /gone\0HEAD ccc\0branch refs/heads/x/y\0prunable gitdir file points to non-existent location\0\0";
        let w = super::parse_worktrees(out);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].branch.as_deref(), Some("main"));
        assert_eq!(w[1].branch, None);
        assert!(w[1].locked && !w[1].prunable);
        assert_eq!(w[2].branch.as_deref(), Some("x/y"));
        assert!(w[2].prunable);

        let repo = TempRepo::new(&[]);
        let wt = repo.root.join("wt");
        repo.git().worktree_add(&wt, "side", "HEAD").expect("add");
        let live = repo.git().worktrees().expect("list");
        assert_eq!(live.len(), 2);
        assert_eq!(live[1].branch.as_deref(), Some("side"));
    }
}
