+++
title = "The architecture model matches the code"
state = "accepted"
scope = ["crates/kitsu/src/**", "app/src-tauri/src/**", ".kitsu/architecture/**", "docs/**"]
+++
Every module in `[architecture] cover` belongs to exactly one component in
`.kitsu/architecture/`, every path a component claims exists, and every
reference resolves. A new module comes with its component, or with an edit
to an existing one, in the same change.

Why: architecture docs rot silently. The prototype of this model, written
from the code a few commits earlier, already missed `index.rs` and
`mcp.rs` when it was first checked.

The check can't tell whether the prose is still true. It keeps the claims
the prose rests on honest, and a component whose files changed shape is
the place to re-read it.

Enforced by `architecture`.
