+++
title = "Find why freshness_follows_scope failed once"
scope = ["crates/kitsu/src/check.rs", "crates/kitsu/src/git.rs"]
checks = ["test"]
+++
`check::tests::freshness_follows_scope` failed once in a full `cargo test`
run on 2026-09-24 while six other processes were cloning repositories
(heavy CPU and disk). It passed alone and in 95 reruns, 30 of them under
synthetic CPU and disk load. The panic message was lost (output was
filtered) and the asserts didn't print the status they saw; they do now.

Not "flaky" until explained. Hypotheses, none confirmed:
- `worktree_tree` copies the index, so the copy has a fresh mtime. Git's
  racy-clean check compares entry mtimes with the index file's mtime, so a
  same-size rewrite within one filesystem timestamp tick could be missed.
  Needs the write and the original index write in the same tick; unlikely
  in this test, but a real hole for fast agents. Preserving the original
  mtime on the copy would close it.
- A check `execute` hitting a timeout under load and recording evidence
  on a different tree than expected.

Done when the cause is shown, or the next failure's message is captured
and explains it.
