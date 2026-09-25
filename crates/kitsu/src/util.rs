use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// Short content identity: the first 16 hex chars of SHA-256. Used for
/// provenance and artifact names, not for security.
pub fn content_id(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex(&digest[..8])
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Wall-clock milliseconds. Only ever used for display and for "since you
/// left"; ordering always comes from sequence numbers.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Short, human-typable random id like `r4f7kq`. Uniqueness is enforced by
/// the database primary key; callers retry on collision.
pub fn short_id(prefix: char) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.write_u32(std::process::id());
    let mut n = h.finish();
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut s = String::with_capacity(7);
    s.push(prefix);
    for _ in 0..6 {
        s.push(ALPHABET[(n % 32) as usize] as char);
        n /= 32;
    }
    s
}

pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "untitled".into()
    } else {
        out
    }
}

pub fn ago(then_ms: i64, now: i64) -> String {
    let s = ((now - then_ms).max(0) / 1000) as u64;
    match s {
        0..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

/// Is `path` inside `dir` (or `dir` itself), by the path text alone?
///
/// Both must be absolute; a relative path is never inside. On Windows that
/// includes `\foo` (rooted on the current drive) and `C:foo` (relative to
/// C:'s current directory). There, names compare ignoring case, `\\?\C:\x`
/// is `C:\x`, `/` and `\` are the same, and trailing dots and spaces are
/// dropped from names the way Windows does (`.. ` is `..`). `..` is applied
/// to the text; symlinks are `resolves_within`'s business.
pub fn lexically_within(path: &Path, dir: &Path) -> bool {
    match (path_key(path), path_key(dir)) {
        (Some(p), Some(d)) => p.starts_with(&d),
        _ => false,
    }
}

/// `lexically_within`, then the same after resolving symlinks and
/// junctions: the deepest part of `path` that exists is canonicalized, and
/// so is `dir`. A `dir` that doesn't exist can only be judged by its text.
pub fn resolves_within(path: &Path, dir: &Path) -> bool {
    if !lexically_within(path, dir) {
        return false;
    }
    let Ok(real_dir) = std::fs::canonicalize(dir) else {
        return true;
    };
    let mut probe = path.to_path_buf();
    let mut rest = Vec::new();
    let real = loop {
        if let Ok(r) = std::fs::canonicalize(&probe) {
            break r;
        }
        match probe.file_name() {
            Some(n) => rest.push(n.to_os_string()),
            None => return false,
        }
        if !probe.pop() {
            return false;
        }
    };
    let real: PathBuf = real.join(rest.iter().rev().collect::<PathBuf>());
    lexically_within(&real, &real_dir)
}

/// A relative path as the repository spells it: `/` between names, on
/// Windows too (`src\a.rs` is `src/a.rs`), the way git and globs see it.
pub fn repo_path(rel: &Path) -> String {
    rel.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Tests: make `link` a directory link to `target`. A junction on Windows,
/// which needs no privilege (a symlink needs Developer Mode or elevation)
/// and is followed the same way.
#[cfg(test)]
pub(crate) fn link_dir(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("symlink");
    #[cfg(windows)]
    {
        let st = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("cmd");
        assert!(st.success(), "mklink /J {}", link.display());
    }
}

/// The comparable spelling of an absolute path: its root, then its names.
fn path_key(p: &Path) -> Option<Vec<String>> {
    if !p.is_absolute() {
        return None;
    }
    let mut key = Vec::new();
    let mut root = 0;
    // `\\?\` paths reach the file system as written: no trimming there.
    let mut verbatim = false;
    let up = |key: &mut Vec<String>, root: usize| {
        if key.len() > root {
            key.pop();
        }
    };
    for c in p.components() {
        match c {
            Component::Prefix(pre) => {
                verbatim = pre.kind().is_verbatim();
                key.push(prefix_key(pre.kind()));
                root = key.len();
            }
            Component::RootDir => {
                key.push(std::path::MAIN_SEPARATOR.to_string());
                root = key.len();
            }
            Component::CurDir => {}
            Component::ParentDir => up(&mut key, root),
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if cfg!(windows) && verbatim {
                    key.push(name.to_lowercase());
                } else if cfg!(windows) {
                    if name.trim_end_matches(' ') == ".." {
                        up(&mut key, root);
                        continue;
                    }
                    let n = name.trim_end_matches([' ', '.']);
                    if !n.is_empty() {
                        key.push(n.to_lowercase());
                    }
                } else {
                    key.push(name.into_owned());
                }
            }
        }
    }
    Some(key)
}

fn prefix_key(p: Prefix) -> String {
    let low = |s: &std::ffi::OsStr| s.to_string_lossy().to_lowercase();
    match p {
        Prefix::Disk(d) | Prefix::VerbatimDisk(d) => {
            format!("{}:", d.to_ascii_lowercase() as char)
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            format!(r"\\{}\{}", low(server), low(share))
        }
        Prefix::DeviceNS(d) => format!(r"\\.\{}", low(d)),
        Prefix::Verbatim(v) => format!(r"\\?\{}", low(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(
            slugify("Make payment retries idempotent!"),
            "make-payment-retries-idempotent"
        );
        assert_eq!(slugify("  --  "), "untitled");
        assert_eq!(slugify("Ünïcode & stuff"), "n-code-stuff");
    }

    /// An absolute path on this platform, from a Unix-style one.
    fn abs(p: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:{p}"))
        } else {
            PathBuf::from(p)
        }
    }

    #[test]
    fn inside_a_directory_by_its_text() {
        let wt = abs("/repo/.git/kitsu/worktrees/r1");
        let within = |p: &str| lexically_within(Path::new(p), &wt);
        let abs_within = |p: &str| lexically_within(&abs(p), &wt);
        assert!(abs_within("/repo/.git/kitsu/worktrees/r1/src/a.rs"));
        assert!(abs_within("/repo/.git/kitsu/worktrees/r1"));
        assert!(abs_within("/repo/.git/kitsu/worktrees/r1/./src/../a.rs"));
        assert!(!abs_within(
            "/repo/.git/kitsu/worktrees/r1/../../../../src/a.rs"
        ));
        assert!(!abs_within("/repo/.git/kitsu/worktrees/r10/a.rs"));
        assert!(!abs_within("/home/me/.ssh/config"));
        assert!(!abs_within("/../../repo/x"));
        assert!(!within("src/a.rs"), "relative is never inside");
        assert!(!within(""));
        if cfg!(windows) {
            // Rooted on the current drive, or relative to a drive's cwd.
            assert!(!within("/repo/.git/kitsu/worktrees/r1/a.rs"));
            assert!(!within(r"\repo\.git\kitsu\worktrees\r1\a.rs"));
            assert!(!within(r"C:repo\.git\kitsu\worktrees\r1\a.rs"));
            // Case, separators and verbatim prefixes don't matter.
            assert!(within(r"c:\REPO\.git\Kitsu\worktrees\R1\src\a.rs"));
            assert!(within(r"\\?\C:\repo\.git\kitsu\worktrees\r1\a.rs"));
            assert!(within("C:/repo/.git/kitsu/worktrees/r1/a.rs"));
            assert!(within(r"C:\repo\.git\kitsu\worktrees\r1. \a.rs"));
            // Another drive, a share, and `.. ` (which Windows reads as `..`).
            assert!(!within(r"D:\repo\.git\kitsu\worktrees\r1\a.rs"));
            assert!(!within(r"\\host\share\repo\.git\kitsu\worktrees\r1\a.rs"));
            assert!(!within(r"C:\repo\.git\kitsu\worktrees\r1\.. \x"));
            assert!(lexically_within(
                Path::new(r"\\?\UNC\HOST\share\wt\a"),
                Path::new(r"\\host\share\wt")
            ));
            // Verbatim paths are taken as written: `r1.` is another folder.
            assert!(!within(r"\\?\C:\repo\.git\kitsu\worktrees\r1.\a.rs"));
        } else {
            // Names are exact here.
            assert!(!within("/repo/.git/kitsu/worktrees/R1/a.rs"));
            assert!(!within("/repo/.git/kitsu/worktrees/r1./a.rs"));
        }
    }

    #[test]
    fn repo_paths_use_forward_slashes() {
        let p: PathBuf = ["src", "agent", "host.rs"].iter().collect();
        assert_eq!(repo_path(&p), "src/agent/host.rs");
        assert_eq!(repo_path(Path::new("./a/b")), "a/b");
        if cfg!(windows) {
            assert_eq!(repo_path(Path::new(r"src\a.rs")), "src/a.rs");
        }
    }

    #[test]
    fn a_link_that_points_out_is_outside() {
        let root = std::env::temp_dir().join(format!("kitsu-within-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (wt, away) = (root.join("wt"), root.join("away"));
        std::fs::create_dir_all(wt.join("src")).expect("mkdir");
        std::fs::create_dir_all(&away).expect("mkdir");
        assert!(resolves_within(
            &wt.join("src").join("new").join("a.rs"),
            &wt
        ));
        assert!(!resolves_within(&away.join("a.rs"), &wt));
        let out = wt.join("out");
        link_dir(&away, &out);
        assert!(lexically_within(&out.join("a.rs"), &wt));
        assert!(!resolves_within(&out.join("a.rs"), &wt));
        assert!(!resolves_within(&out.join("deeper").join("a.rs"), &wt));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ids_are_short_and_distinct() {
        let a = short_id('r');
        let b = short_id('r');
        assert_eq!(a.len(), 7);
        assert_ne!(a, b);
    }
}
