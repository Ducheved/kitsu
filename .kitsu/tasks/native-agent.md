+++
title = "Kitsu's own agent loop: compaction, resume, loops, budgets"
scope = ["crates/kitsu/src/agent/**", "crates/kitsu/src/runner.rs", "crates/kitsu/src/agents.rs", "fixtures/stub-model/**"]
checks = ["test"]
+++
v1 base is in (decision `own-loop`): provider (OpenAI chat, streamed,
retries), brain/host over JSON-RPC, the 12 tools, pinned prefix and
ledger, `finish` verified by checks, turn and token budgets, the key
never written. Tests in crates/kitsu/tests/native.rs.

Compaction is in: over 80% of the window, old tool outputs become
pointers, then older steps a digest regenerated from the journal; on the
provider's overflow error only the newest step is kept verbatim and the
request is retried once, or not at all if nothing could shrink. The
harness and the brief are byte-identical in every request.

Resume is in: `kitsu run <task> --agent kitsu --from rX --resume`
continues rX's conversation; a call that began and never ended is
settled from what was recorded before it ran (a write: applied or not by
the file's hash; a command: unknown, never re-run), read-only calls just
run again. Budgets count per run.

Also in: the loop signal (the same call three times in the last eight
with the files unchanged warns once, a second signal stops as
doom_loop); cancel kills a running command and ends the run cancelled;
429/5xx retried with backoff up to five attempts, 401 not at all; paths
confined to the worktree, a symlink made with shell included.

Still to do:
- A live comparison on OpenRouter once the key reaches a session: the
  same tasks with this loop and with Claude Code, verified runs and cost.
- Anthropic Messages as a second provider, when someone needs it.
- A cloud brain over WSS (the host stays local), when someone needs it.
