+++
title = "Commands run under sh on every platform, Git for Windows' sh on Windows"
state = "accepted"
scope = ["crates/kitsu/src/proc.rs", "crates/kitsu/src/check.rs", "crates/kitsu/src/agent/host.rs"]
rejected = [
  "`cmd /C` on Windows: checks are written in POSIX shell, and cmd reads some of them as something that passes (`echo ok; exit 1` is one echo that exits 0). A check that can pass without running what it says is worse than one that can't run",
  "Falling back to cmd when no sh is found: the same false pass, only rarer",
  "PowerShell: a third syntax for the same `kitsu.toml`, and a repository's checks would stop being portable",
  "A Job Object to stop a command's process tree on Windows: it needs `unsafe` (the workspace forbids it) or a new dependency; `taskkill /T /F` does the same for a tree whose root is still running",
]
+++
A check's `run` and the native loop's `shell` tool are `sh -c` commands
everywhere. On Windows the shell is, in order: `KITSU_SH` if set, `sh.exe`
on PATH, then the one Git for Windows ships next to git (Kitsu needs git
anyway). None of those is an error, recorded as the check's `error`
outcome with the reason in its log; never a pass.

Stopping a command stops its tree: the process group on Unix, `taskkill
/T /F` from the root on Windows. On Windows, what a command started and
left behind after its shell exited can't be found from the shell's pid;
Kitsu stops reading its output after two seconds but it keeps running.

Revisit if: Windows users need checks that aren't POSIX shell, or
orphaned background processes on Windows turn out to matter (a Job Object
would catch them).
