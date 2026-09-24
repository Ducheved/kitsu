+++
title = "Memory that knows what kind of thing it remembers and when it went stale"
scope = ["crates/kitsu/src/intent.rs", "crates/kitsu/src/memory.rs", "crates/kitsu/src/brief.rs", "app/src/**"]
checks = ["test", "ui"]
state = "done"
+++
One memory system, typed: fact, gotcha, convention, preference, lesson. Each
note has provenance (who wrote it, from which run) and anchors (paths and
the content hash they had). When an anchor changes, the note is stale until
someone confirms it. Repository memory lives in `.kitsu/memory/`; personal
preferences in the user config dir. Agents propose notes as files; they land
through review like decisions. Briefs include the notes that apply, with why.
