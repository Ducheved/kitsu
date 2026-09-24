import unittest

from fake_upstream import FakeUpstream
from payments import Timeout, charge


class Retries(unittest.TestCase):
    def test_gives_up_when_upstream_is_down(self):
        up = FakeUpstream(always_timeout=True)
        with self.assertRaises(Timeout):
            charge(up, "4242", 1000)
        self.assertLessEqual(up.calls, 3, "retry budget is 3 attempts")

    def test_normal_charge(self):
        up = FakeUpstream()
        self.assertIn("id", charge(up, "4242", 1000))


if __name__ == "__main__":
    unittest.main()
