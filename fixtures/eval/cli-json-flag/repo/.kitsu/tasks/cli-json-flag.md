+++
title = "todo.py list --json for the menu-bar widget"
scope = ["todo.py"]
checks = ["cli"]
+++
The menu-bar widget wants the list as JSON. Add `--json` to `todo.py list`:

- stdout is one JSON array followed by a newline, and nothing else;
- each element is exactly `{"id": <int>, "title": <str>, "done": <bool>}`
  (the file may hold other keys; leave them out), sorted by id;
- an empty or missing file prints `[]`;
- `--json` belongs to `list` only: `todo.py add --json milk` is a usage
  error (argparse's exit status 2), as is `todo.py --json list`;
- without `--json`, `list` prints exactly what it prints today.
