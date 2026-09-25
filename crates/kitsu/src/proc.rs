//! Child processes: which shell runs a command, and stopping a whole tree.
//!
//! Checks and the native loop's `shell` tool are POSIX `sh` commands on
//! every platform (decision `sh-everywhere`). On Windows that is the
//! `sh.exe` Git for Windows ships; with none, running a command is an
//! error, never a silent fallback to `cmd.exe`, which would read
//! `echo ok; exit 1` as one echo and pass it.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

/// Names a shell explicitly, instead of the one found on PATH or next to git.
pub const SH_ENV: &str = "KITSU_SH";

/// The shell `sh -c` commands run under, or why there is none.
pub fn sh() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os(SH_ENV).filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        return if p.is_file() {
            Ok(p)
        } else {
            Err(format!("{SH_ENV} is {}, which is not a file", p.display()))
        };
    }
    static FOUND: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    FOUND.get_or_init(find_sh).clone()
}

#[cfg(not(windows))]
fn find_sh() -> Result<PathBuf, String> {
    // Resolved by the OS as it always was.
    Ok(PathBuf::from("sh"))
}

#[cfg(windows)]
fn find_sh() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join("sh.exe"))
        .find(|p| p.is_file())
    {
        return Ok(p);
    }
    // Git for Windows is required anyway; its sh.exe is not always on PATH.
    let exec_path = std::process::Command::new("git")
        .arg("--exec-path")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    if let Some(p) = exec_path.as_deref().and_then(git_sh) {
        return Ok(p);
    }
    Err(format!(
        "no POSIX shell: checks and shell commands run under `sh -c`, and there is no sh.exe on PATH or in Git for Windows' folder; install Git for Windows or set {SH_ENV} to a sh.exe"
    ))
}

/// Git for Windows' `sh.exe`, from git's exec path
/// (`<git>\mingw64\libexec\git-core`). `bin\sh.exe` first: it puts the
/// POSIX tools on PATH before starting the real shell.
#[cfg_attr(not(windows), allow(dead_code))]
fn git_sh(exec_path: &Path) -> Option<PathBuf> {
    exec_path
        .ancestors()
        .skip(1)
        .take(4)
        .flat_map(|root| {
            [
                root.join("bin").join("sh.exe"),
                root.join("usr").join("bin").join("sh.exe"),
            ]
        })
        .find(|p| p.is_file())
}

/// Kill `pid` and everything it started. Call it while `pid` is still
/// running (not yet waited for): on Unix it is a process group made with
/// `process_group(0)`, on Windows the tree `taskkill /T` finds from it. A
/// Windows process whose parent already exited can't be found this way.
pub fn kill_tree_blocking(pid: u32) {
    let _ = killer(pid)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// `kill_tree_blocking` for async callers.
pub async fn kill_tree(pid: u32) {
    let _ = tokio::process::Command::from(killer(pid))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

fn killer(pid: u32) -> std::process::Command {
    #[cfg(windows)]
    {
        // From the system directory, not whatever PATH says taskkill is.
        let exe = std::env::var_os("SystemRoot")
            .map(|r| PathBuf::from(r).join("System32").join("taskkill.exe"))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from("taskkill"));
        let mut c = std::process::Command::new(exe);
        c.args(["/T", "/F", "/PID", &pid.to_string()]);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = std::process::Command::new("kill");
        c.args(["-KILL", "--", &format!("-{pid}")]);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_for_windows_layouts_are_found() {
        let root = std::env::temp_dir().join(format!("kitsu-gitsh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let exec = root.join("mingw64/libexec/git-core");
        std::fs::create_dir_all(&exec).expect("mkdir");
        assert_eq!(git_sh(&exec), None);
        std::fs::create_dir_all(root.join("usr/bin")).expect("mkdir");
        std::fs::write(root.join("usr/bin/sh.exe"), "").expect("sh");
        assert_eq!(
            git_sh(&exec),
            Some(root.join("usr").join("bin").join("sh.exe"))
        );
        std::fs::create_dir_all(root.join("bin")).expect("mkdir");
        std::fs::write(root.join("bin/sh.exe"), "").expect("sh");
        assert_eq!(git_sh(&exec), Some(root.join("bin").join("sh.exe")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_shell_is_found_and_it_runs_posix_syntax() {
        let sh = sh().expect("a shell");
        let out = std::process::Command::new(sh)
            .args(["-c", "echo ok; exit 3"])
            .output()
            .expect("sh");
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "ok\n");
    }
}
