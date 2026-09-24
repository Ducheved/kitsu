+++
title = "A change is judged by your rules, not the ones it brings"
scope = ["crates/kitsu/src/integrate.rs", "crates/kitsu/src/status.rs"]
checks = ["test"]
+++
Accept reads checks and invariants from the main worktree, which the agent
never writes. Edits to `.kitsu/**` or protected paths inside the change are
shown as rule changes and need an approval bound to a hash of exactly that
diff.

Covered by `weakening_a_protected_test_needs_approval_bound_to_that_diff`.
