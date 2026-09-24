+++
title = "Show rule changes as their own review step"
scope = ["app/src/components/ReviewCard.svelte", "crates/kitsu/src/integrate.rs"]
checks = ["ui", "test"]
+++
When an agent proposes a decision or edits a check, the reviewer
should see "this changes the rule X from A to B" above the code diff, not a
Markdown diff mixed into the file list.
