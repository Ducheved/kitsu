+++
title = "Missing evidence is never reported as passing"
state = "accepted"
scope = ["crates/kitsu/src/check.rs", "crates/kitsu/src/status.rs", "crates/kitsu/src/recover.rs", "app/src/**"]
+++
No evidence means "not run". Evidence for another tree means "stale" unless
the check's declared scope is untouched. A check that changed files while
running is recorded but bound to no tree. A run whose owner vanished is
"interrupted", not failed and not finished. Nothing times out into success.

The UI must say these words too. A green badge on "not run" is the same
bug as a fallback in the store.

Enforced by `test`.
