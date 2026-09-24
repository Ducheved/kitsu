+++
title = "Git adapter"
level = "component"
parent = "cli"
paths = ["crates/kitsu/src/git.rs"]
uses = ["shared"]
+++
Every git operation, through the git CLI (never libgit2). Snapshots use a
scratch index so the agent's own index flags can't hide files.
Never touches your checkout's index or HEAD except in accept.
