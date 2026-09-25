import unittest

from cache import TTLCache


class Clock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        return self.now


def make(maxsize=2, ttl=60):
    clock = Clock()
    return TTLCache(maxsize, ttl=ttl, clock=clock), clock


class HeldOut(unittest.TestCase):
    def test_get_makes_an_entry_most_recently_used(self):
        c, _ = make()
        c.set("a", 1)
        c.set("b", 2)
        self.assertEqual(c.get("a"), 1)
        c.set("c", 3)
        self.assertEqual((c.get("a"), c.get("b"), c.get("c")), (1, None, 3))

    def test_expired_entries_go_before_live_ones(self):
        c, clock = make()
        c.set("a", 1)          # expires at +60
        clock.now += 50
        c.set("b", 2)          # expires at +110
        self.assertEqual(c.get("a"), 1)   # a is now the most recently used
        clock.now += 11        # a has expired, b is live
        c.set("c", 3)
        self.assertEqual((c.get("b"), c.get("c")), (2, 3))

    def test_get_does_not_extend_the_ttl(self):
        c, clock = make()
        c.set("a", 1)
        clock.now += 50
        self.assertEqual(c.get("a"), 1)
        clock.now += 11
        self.assertIsNone(c.get("a"))

    def test_set_again_resets_the_ttl(self):
        c, clock = make()
        c.set("a", 1)
        clock.now += 50
        c.set("a", 2)
        clock.now += 50
        self.assertEqual(c.get("a"), 2)

    def test_len_counts_live_entries(self):
        c, clock = make(maxsize=5)
        c.set("a", 1)
        c.set("b", 2)
        clock.now += 30
        c.set("c", 3)
        self.assertEqual(len(c), 3)
        clock.now += 31
        self.assertEqual(len(c), 1)

    def test_default(self):
        c, clock = make()
        self.assertEqual(c.get("x", "none"), "none")
        c.set("x", 1)
        clock.now += 61
        self.assertEqual(c.get("x", "none"), "none")

    def test_never_over_maxsize(self):
        c, _ = make(maxsize=3)
        for i in range(10):
            c.set(i, i)
        self.assertEqual(len(c), 3)
        self.assertEqual([c.get(i) for i in range(10)], [None] * 7 + [7, 8, 9])


if __name__ == "__main__":
    unittest.main()
