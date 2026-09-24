+++
title = "One kitsu process owns one run"
rejected = [
  "Runs owned by the desktop app: closing the window would kill agents (VS Code moved its agent host out of the window for this reason, Aug 2026)",
  "A long-lived daemon: another process to install, secure and version; nothing in the single-user case needs it yet",
  "Heartbeats in the database for liveness: writes caused only by time passing, and still wrong after a pause",
]
+++
`kitsu run` records the intent, creates the worktree, supervises the agent,
snapshots and verifies. The app starts runs by launching the same command.
Custody is an OS file lock held for the process lifetime; `stop` and
answers wake the owner with SIGUSR1, with a 1 Hz poll as the safety net.

Measured: ~7 MB RSS per supervisor, ~50 ms from launch to a running agent,
100 idle supervisors at 1.2% of one core.

Revisit if: we need runs on another machine (the process boundary is
already the seam), or Windows needs a different wake mechanism.
