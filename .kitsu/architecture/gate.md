+++
title = "Hooks and CI gate"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/contract.rs", "crates/kitsu/src/hooks.rs"]
uses = ["evidence", "status", "rules", "integration", "store", "workspace", "gitio", "shared"]
+++
The same contract enforced where agents already run: `kitsu gate` inside
Claude Code's, Codex's and Cursor's hooks, `kitsu ci` on a pull request,
`kitsu diff` for a person. It judges the diff by content against the base's
rules, never tool calls or what the agent says. It reuses the evidence
store, so a check that already ran on these exact files doesn't run again.
It never writes intent, and hook configs change only through
`kitsu hooks install` / `uninstall`, which merge rather than replace.
