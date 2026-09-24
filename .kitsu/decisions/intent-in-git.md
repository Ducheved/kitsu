+++
title = "Intent lives in the repo as files; execution lives in SQLite"
rejected = [
  "Everything in one database: tasks and rules would stop being reviewable, branchable and readable without Kitsu",
  "Everything in files: run lifecycles need transactions and concurrent writers",
  "A graph database or event sourcing: no consumer needs either; relational tables and derived views cover every query we make",
  "A vector store for 'memory': the things that must survive are typed (rule, decision, question, evidence), and none of them is found by similarity",
]
+++
Tasks, decisions and questions are Markdown with TOML front
matter under `.kitsu/`. Git versions, branches, merges and reviews them with
the code they describe. Delete Kitsu and they're still readable.

Runs, evidence, events and asks are machine state in
`<git-common-dir>/kitsu/state.db`: local, transactional, never committed.

Revisit if: teams need to share run history across machines (then it's a
sync protocol for the store, not a reason to move intent).
