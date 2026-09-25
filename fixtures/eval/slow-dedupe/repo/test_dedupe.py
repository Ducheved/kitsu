import time
import unittest

from dedupe import dedupe


class Dedupe(unittest.TestCase):
    def test_keeps_first_occurrence_in_order(self):
        self.assertEqual(dedupe([3, 1, 3, 2, 1]), [3, 1, 2])

    def test_fast_enough_for_the_nightly_import(self):
        items = [i % 15000 for i in range(30000)]
        start = time.perf_counter()
        out = dedupe(items)
        took = time.perf_counter() - start
        self.assertEqual(out, list(range(15000)))
        self.assertLess(took, 0.5, f"took {took:.2f}s")


if __name__ == "__main__":
    unittest.main()
