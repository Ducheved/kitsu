+++
title = "Under --policy auto, a shell command the screen catches still waits for you"
scope = ["crates/kitsu/src/agent/host.rs"]
state = "accepted"
rejected = [
  "Refusing it outright (nobody may be watching an auto run): the policy code already has an answer for what auto doesn't cover, and it is to ask; an ACP agent's request for a path outside the worktree waits for a human under auto too",
  "Screening paths under auto as triage does: /usr, /tmp and absolute paths show up in harmless commands, and auto already lets commands reach them; the screen here is for effects that land outside the worktree for certain",
  "Calling this a sandbox: it is the same lexical screen as triage's (decision `no-sandbox-yet`); a command can reach the network or your refs in ways no word list sees",
]
+++
`--policy auto` runs shell commands in the worktree on their own, except
the ones `screen` catches without its path checks: the network, git remotes
and configuration, git refs that every worktree shares (`update-ref`,
`branch`/`tag` with a name, `stash`, `worktree`, `checkout -b`, `gc`, ...),
package installs, more privileges, the git directory. Those are asked, with
the reason on the ask card, and a "no" is recorded as `denied`, like any
other declined command.

A worktree has its own files, not its own refs, config or network: a
`git push` or `git update-ref refs/heads/main` from it lands in your
repository. Auto is still not a sandbox; it only stops asking about what
stays in the worktree.

A call that began before a crash is settled from the journal before any
policy runs, so resume never asks about a command it won't run, and a
declined ask never records "not run" for one that had started.
