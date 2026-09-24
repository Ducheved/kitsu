+++
title = "Checks say what they guard; there are no invariants"
state = "accepted"
scope = ["crates/kitsu/src/intent.rs", "crates/kitsu/src/status.rs", "crates/kitsu/src/brief.rs"]
rejected = [
  "Invariants as their own kind of file: one more word to learn, and most of them were prose pointing at `test`, which the prose couldn't enforce",
  "Making every scoped check required: `scope` says when evidence goes stale, which is not the same as when a check is required (fmt is stale on any .rs edit, but a task may not need it)",
]
+++
A check in `.kitsu/kitsu.toml` can say `guards = [paths]` and `why = "..."`.
Any change that touches those paths needs the check to pass, whatever the
task says; the `why` goes into the brief under "Done means", the part that
stays in the agent's system prompt through compaction.

What the old invariant files said in prose is now in decisions with the
same names. A file left in `.kitsu/invariants/` is reported as a problem,
not skipped: it was a rule, and silently dropping a rule is the failure
this repository exists to prevent.
