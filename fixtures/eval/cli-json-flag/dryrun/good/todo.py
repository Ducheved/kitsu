"""A tiny todo list: python3 todo.py [--file F] add TITLE | done ID | list [--json]"""

import argparse
import json
import os
import sys


def load(path):
    if not os.path.exists(path):
        return []
    with open(path) as f:
        return json.load(f)


def save(path, items):
    with open(path, "w") as f:
        json.dump(items, f, indent=1)


def main(argv=None):
    p = argparse.ArgumentParser(prog="todo")
    p.add_argument("--file", default=os.environ.get("TODO_FILE", "todos.json"))
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("add").add_argument("title")
    sub.add_parser("done").add_argument("id", type=int)
    sub.add_parser("list").add_argument("--json", action="store_true", help="print a JSON array")
    args = p.parse_args(argv)

    items = load(args.file)
    if args.cmd == "add":
        new_id = max((i["id"] for i in items), default=0) + 1
        items.append({"id": new_id, "title": args.title, "done": False})
        save(args.file, items)
        print(new_id)
    elif args.cmd == "done":
        for i in items:
            if i["id"] == args.id:
                i["done"] = True
                save(args.file, items)
                return 0
        print(f"no todo {args.id}", file=sys.stderr)
        return 1
    elif args.json:
        rows = [{"id": i["id"], "title": i["title"], "done": bool(i["done"])}
                for i in sorted(items, key=lambda i: i["id"])]
        print(json.dumps(rows))
    else:
        for i in sorted(items, key=lambda i: i["id"]):
            print(f"[{'x' if i['done'] else ' '}] {i['id']:>3}  {i['title']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
