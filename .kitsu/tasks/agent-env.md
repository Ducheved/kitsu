+++
title = "Don't hand the agent every variable in Kitsu's environment"
scope = ["crates/kitsu/src/runner.rs", "crates/kitsu/src/agents.rs"]
checks = ["test"]
+++
Agent processes inherit Kitsu's whole environment. The compaction probe
showed what that means when Kitsu itself runs inside another tool: the
host's session tokens, account ids and settings reached Claude Code, and
one of them (`CLAUDE_CODE_REMOTE`) changed when it compacts.

Proposal: start agents with an allowlist (PATH, HOME, USER, LANG, TERM,
SHELL, TMPDIR, XDG dirs, proxy and certificate variables) plus the agent's
own `env` from agents.toml, plus `pass_env = [...]` for anything else a
user wants through. Provider keys (ANTHROPIC_API_KEY, OPENAI_API_KEY, ...)
pass only when named in the agent's config.

Risk: breaking setups that relied on inheritance. Done when a test shows a
variable set for Kitsu but not allowed doesn't reach the test agent, and
`kitsu agents` lists what each agent receives.
