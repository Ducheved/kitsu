import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


def todo(*args, items=None):
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "todos.json")
        if items is not None:
            with open(path, "w") as f:
                json.dump(items, f)
        return subprocess.run([sys.executable, os.path.join(HERE, "todo.py"), "--file", path, *args],
                              capture_output=True, text=True, timeout=30)


class Cli(unittest.TestCase):
    def test_list_json(self):
        items = [{"id": 1, "title": "milk", "done": False}, {"id": 2, "title": "bread", "done": True}]
        r = todo("list", "--json", items=items)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout), items)

    def test_list_text(self):
        r = todo("list", items=[{"id": 1, "title": "milk", "done": False}])
        self.assertEqual(r.stdout, "[ ]   1  milk\n")


if __name__ == "__main__":
    unittest.main()
