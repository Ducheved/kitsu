+++
title = "Coding agent (Claude Code, Codex, OpenCode, ...)"
level = "external"
technology = "ACP over stdio"
uses = [{ to = "cli", why = "asks permission, calls Kitsu's read-only MCP tools" }]
+++
Does the work in a run's worktree. Replaceable: nothing Kitsu knows lives
only in its context.
