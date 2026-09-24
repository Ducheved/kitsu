+++
title = "Use the git binary, not a git library"
rejected = [
  "libgit2 / gitoxide: a second implementation of the user's git, with its own idea of config, hooks and worktrees",
]
+++
Every git operation is a process spawn (a few ms). None of them is on a
typing path. Status refresh no longer spawns git at all (snapshot facts are
stored once). Kitsu's own commits run with hooks disabled; accepted commits
carry your identity.

Revisit if: a hot path needs git per keystroke or per event.
