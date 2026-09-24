+++
title = "Typed judgments (Jev) for the few decisions that are really semantic"
scope = ["crates/kitsu/src/judge.rs"]
checks = ["test"]
+++
A small interface for yes/no, choice and score questions with calibrated
probabilities, backed by TypeSafe's System One API when configured and by
nothing otherwise. Every judgment is stored with its inputs hash,
probabilities, model and cost. A failed or missing judgment is UNKNOWN and
the caller takes the conservative path.
