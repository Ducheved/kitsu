# Working on Kitsu

If you're an agent started by Kitsu, your brief already has what follows.
If you're not, read this.

- What's being worked on, what must stay true and what was decided (and
  rejected) is in `.kitsu/`. Read the decisions and the checks whose
  `guards` cover the files you touch before you change them.
- Checks are defined in `.kitsu/kitsu.toml`. `kitsu check` runs them;
  without kitsu: `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace`, `cargo run -q
  -p kitsu --bin kitsu -- arch check`, and in `app/`: `npm run check &&
  npm test && npm run build`.
- A new module needs a component in `.kitsu/architecture/` (or a line in an
  existing one's `paths`); `kitsu arch` shows the model.
- Don't weaken a test or a check to get green. Changes under `.kitsu/`,
  `crates/kitsu/tests/` and `fixtures/` are reviewed as rule changes.
- If you need a decision, write `.kitsu/questions/<name>.md` and stop.
