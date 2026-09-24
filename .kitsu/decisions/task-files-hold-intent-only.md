+++
title = "A task file says what should happen, never what happened"
state = "accepted"
scope = ["crates/kitsu/src/intent.rs"]
+++
Task front matter is `title`, `state`, `scope`, `checks`, `after`.
Attempts, status, verdicts, run ids and lessons live in the store or in
memory notes. A new task field needs a decision. The only writes from
execution into `.kitsu/` are `done` on accept-and-close and `answered` on
answer, both on a human's command (check `intent-writers`).

Transcript-as-state (a memory bank's "next steps", a chat history that is
the workflow's only state) is how plan state turns into stale authority.
Pinned by `tests/separation.rs`.

Enforced by `test`.
