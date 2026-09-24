+++
title = "No OS sandbox in the first cut, and the UI says so"
rejected = [
  "Calling the permission prompts a sandbox",
  "Shipping bubblewrap now: Ubuntu 24.04 blocks unprivileged user namespaces unless an AppArmor profile is installed, which needs root",
]
+++
Worktrees isolate changes, not processes. Agents run with your user's
permissions and can reach anything you can. Approvals are UX, not
containment, and the ask card says that in plain words.

Next: bubblewrap + seccomp on Linux, sandbox-exec on macOS, following what
Codex and Claude Code ship; see the `sandbox-linux` task.
