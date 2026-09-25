import time
import unittest

from dedupe import dedupe


class HeldOut(unittest.TestCase):
    def test_rows_as_dicts(self):
        rows = [{"id": "1", "name": "a"}, {"id": "2", "name": "b"},
                {"name": "a", "id": "1"}, {"id": "3", "name": "c"}, {"id": "2", "name": "b"}]
        self.assertEqual(dedupe(rows), [{"id": "1", "name": "a"}, {"id": "2", "name": "b"},
                                        {"id": "3", "name": "c"}])

    def test_dict_values_can_be_any_hashable(self):
        rows = [{"k": (1, 2)}, {"k": None}, {"k": (1, 2)}, {"k": 1.5}, {"k": None}]
        self.assertEqual(dedupe(rows), [{"k": (1, 2)}, {"k": None}, {"k": 1.5}])

    def test_order_and_equality_like_before(self):
        self.assertEqual(dedupe(["b", "a", "b", "c", "a"]), ["b", "a", "c"])
        self.assertEqual(dedupe([]), [])
        out = dedupe([1, 1.0, True, 2])
        self.assertEqual(out, [1, 2])
        self.assertIs(type(out[0]), int, "the first occurrence is kept")

    def test_returns_a_new_list(self):
        items = [1, 2, 1]
        out = dedupe(items)
        self.assertEqual(items, [1, 2, 1])
        self.assertIsInstance(out, list)

    def test_fast_on_100k_rows(self):
        # A quick probe first, so a quadratic version fails in a second, not an hour.
        probe = [{"id": str(i % 2500)} for i in range(5000)]
        start = time.perf_counter()
        dedupe(probe)
        self.assertLess(time.perf_counter() - start, 0.25, "5k rows took too long; 100k would take hours")
        rows = [{"id": str(i % 50000), "name": f"n{i % 50000}"} for i in range(100000)]
        start = time.perf_counter()
        out = dedupe(rows)
        took = time.perf_counter() - start
        self.assertEqual(len(out), 50000)
        self.assertEqual(out[:2], [{"id": "0", "name": "n0"}, {"id": "1", "name": "n1"}])
        self.assertLess(took, 2.0, f"took {took:.2f}s")


if __name__ == "__main__":
    unittest.main()
