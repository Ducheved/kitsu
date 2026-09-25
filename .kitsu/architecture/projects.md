+++
title = "Projects list"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/workspaces.rs"]
uses = ["workspace", "status", "rules", "store", "gitio", "shared"]
+++
The repositories the window shows side by side (`workspaces.toml` in the
config dir), a cheap per-repository summary for the switcher, and the
read-only branches and worktrees view. Holds only the list: each
repository's intent, state and trust stay where they were, and removing
an entry never touches its files.
