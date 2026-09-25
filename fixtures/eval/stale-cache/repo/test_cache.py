import unittest

from cache import TTLCache


class Clock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        return self.now


class Cache(unittest.TestCase):
    def test_fresh_entry(self):
        c = TTLCache(10, ttl=60, clock=Clock())
        c.set("sku1", 9.99)
        self.assertEqual(c.get("sku1"), 9.99)

    def test_expired_entry_is_gone(self):
        clock = Clock()
        c = TTLCache(10, ttl=60, clock=clock)
        c.set("sku1", 9.99)
        clock.now += 61
        self.assertIsNone(c.get("sku1"))


if __name__ == "__main__":
    unittest.main()
