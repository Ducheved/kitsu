+++
title = "Many projects in one window: every command names its repository"
state = "proposed"
scope = ["app/src-tauri/src/commands.rs", "app/src/lib/api.ts", "app/src/lib/app.svelte.ts", "crates/kitsu/src/workspaces.rs"]
rejected = [
  "A current repository in the app's state that a switch changes: a command sent just before the switch (an accept, a save) lands in the other repository",
  "One window per repository: what needs you in the others is out of sight, which is the problem the switcher exists for",
  "Tasks and runs spanning repositories: an accept can't move two repositories atomically and one check can't verify two trees",
  "Keeping the list inside one repository's `.kitsu/`: it's about your machine, not about that project, and it would travel with every clone",
]
+++
The list of projects is `workspaces.toml` in the Kitsu config dir, written
only by the app and `kitsu workspaces add|remove`. Everything about one
project stays in that project: intent in `.kitsu/`, runs under its git dir,
trust per repository. Adding a project doesn't trust it.

Every Tauri command that touches a repository takes `repo`, the id of a
listed entry (derived from its root, never a path the window supplies);
an unlisted id is refused. On the UI side each component takes an API bound
to one id when it mounts and is remounted on a switch, so a reply or a
follow-up call can't drift into another project.

Covered by `every_repo_command_names_its_repo` (no command without `repo`
except the list's own) and `a_command_for_one_repo_never_touches_another`
in `app/src-tauri/src/commands.rs`.

The switcher's counts come from `workspace_overview`, which the window
polls. Measured on 5 repositories (release build): 16 ms p50 recomputed,
1 ms p50 when nothing changed, served from a cache keyed by a `stat`-only
fingerprint (`cargo run --release -p kitsu --example scale -- overview`).
