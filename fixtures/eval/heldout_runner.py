"""Runs test files against a scored tree and prints one JSON line.

    python3 -I heldout_runner.py TEST_FILE...     (cwd = the scored tree)

Run with -I, so neither the environment nor the tree can put modules in
front of the standard library: the tree is appended to sys.path, not
prepended, and a `unittest.py` or `decimal.py` an agent left behind can't
stand in for the real one. Test files are loaded by path.
"""

import importlib.util
import io
import json
import os
import sys
import unittest


def main(paths):
    sys.path.append(os.getcwd())
    suite = unittest.TestSuite()
    load_errors = []
    for i, path in enumerate(paths):
        name = f"_scored_{i}_{os.path.splitext(os.path.basename(path))[0]}"
        try:
            spec = importlib.util.spec_from_file_location(name, path)
            mod = importlib.util.module_from_spec(spec)
            sys.modules[name] = mod
            spec.loader.exec_module(mod)
            suite.addTests(unittest.defaultTestLoader.loadTestsFromModule(mod))
        except Exception as e:  # an import error is a failure of every test in it
            load_errors.append({"test": os.path.basename(path), "error": f"{type(e).__name__}: {e}"})
    out = io.StringIO()
    result = unittest.TextTestRunner(stream=out, verbosity=0).run(suite)
    failed = [{"test": t.id().split(".", 1)[-1], "error": tb.strip().splitlines()[-1][:300]}
              for t, tb in result.failures + result.errors]
    report = {
        "ran": result.testsRun,
        "failed": load_errors + failed,
        "ok": not load_errors and result.wasSuccessful() and result.testsRun > 0,
    }
    print(json.dumps(report))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
