+++
title = "Catch architecture drift the path check can't see, and give docs their own tasks"
scope = ["crates/kitsu/src/arch.rs", "crates/kitsu/src/brief.rs", "crates/kitsu/src/intent.rs"]
checks = ["test", "architecture"]
after = ["architecture-model"]
+++
What `kitsu arch check` doesn't catch yet, each behind its own gate:

- Declared `uses` against real dependencies. For Rust, `use crate::` per
  module gives them exactly; other languages need a per-language
  extractor. Gate: a hand-made model misses or invents a dependency that
  matters (for Kitsu's own model the first draft had `run.rs` in the wrong
  component, which made store → runs → store a cycle).
- Structural staleness: flag a component's prose when its file set or its
  dependencies changed, not when any line did. Gate: fewer false flags
  than line-based staleness on this repository's history.
- A C4 slice in the brief: the components a task's scope touches, their
  "must never" lines and their neighbours. Gate: the compaction probe or a
  live run shows an agent crossing a seam the slice would have named.
- `kind = "docs"` tasks whose done means is "the prose of these elements
  matches the code at this tree", reviewed like any other change.
- LikeC4 and Mermaid as exports only; the files stay the source.
