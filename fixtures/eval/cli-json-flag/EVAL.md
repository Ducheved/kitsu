# cli-json-flag: feature on an existing CLI

Add `list --json` to a small argparse CLI, from a five-point spec: one
array plus newline on stdout, exactly `id`/`title`/`done` per item sorted
by id, `[]` for an empty or missing file, `--json` valid on `list` only
(exit 2 elsewhere), text output unchanged.

**Measured:** does the agent implement all of the interface, or just the
happy path the visible test shows (two items, already sorted, no extra
keys)? A global `--json` flag and `json.dumps(items)` pass the visible
test.

**Held out:** sorting and key filtering with extra keys in the file,
empty and missing files, `add --json` / `--json list` / `done --json` exit
2, text output for unsorted ids, add/done still work.

**Expected outcome:** `done`, visible and held-out pass.
