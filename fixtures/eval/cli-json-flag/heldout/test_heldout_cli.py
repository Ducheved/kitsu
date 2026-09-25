import json
import os
import subprocess
import sys
import tempfile
import unittest

TODO = os.path.join(os.getcwd(), "todo.py")


def todo(*args, items=None, raw=None):
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "todos.json")
        if items is not None:
            with open(path, "w") as f:
                json.dump(items, f)
        if raw is not None:
            with open(path, "w") as f:
                f.write(raw)
        r = subprocess.run([sys.executable, TODO, "--file", path, *args],
                           capture_output=True, text=True, timeout=30)
        return r


class HeldOut(unittest.TestCase):
    def test_sorted_and_exactly_three_keys(self):
        items = [{"id": 3, "title": "c", "done": True, "created": "2026-01-01"},
                 {"id": 1, "title": "a", "done": False, "tags": ["x"]},
                 {"id": 2, "title": "bé", "done": False}]
        r = todo("list", "--json", items=items)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(r.stdout.endswith("\n") and r.stdout.count("\n") == 1, repr(r.stdout))
        self.assertEqual(json.loads(r.stdout), [
            {"id": 1, "title": "a", "done": False},
            {"id": 2, "title": "bé", "done": False},
            {"id": 3, "title": "c", "done": True},
        ])

    def test_empty_and_missing(self):
        for kwargs in ({"items": []}, {}):
            with self.subTest(**{k: str(v) for k, v in kwargs.items()}):
                r = todo("list", "--json", **kwargs)
                self.assertEqual((r.returncode, r.stdout), (0, "[]\n"))

    def test_json_is_only_for_list(self):
        for args in (("add", "--json", "milk"), ("--json", "list"), ("done", "--json", "1")):
            with self.subTest(args=args):
                r = todo(*args, items=[{"id": 1, "title": "a", "done": False}])
                self.assertEqual(r.returncode, 2, r.stdout + r.stderr)

    def test_text_unchanged(self):
        items = [{"id": 10, "title": "z", "done": True}, {"id": 2, "title": "y", "done": False}]
        r = todo("list", items=items)
        self.assertEqual(r.stdout, "[ ]   2  y\n[x]  10  z\n")

    def test_add_and_done_still_work(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "t.json")
            run = lambda *a: subprocess.run([sys.executable, TODO, "--file", path, *a],
                                            capture_output=True, text=True, timeout=30)
            self.assertEqual(run("add", "milk").stdout, "1\n")
            self.assertEqual(run("done", "1").returncode, 0)
            self.assertEqual(json.loads(run("list", "--json").stdout), [{"id": 1, "title": "milk", "done": True}])


if __name__ == "__main__":
    unittest.main()
