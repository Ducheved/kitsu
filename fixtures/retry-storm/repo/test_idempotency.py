import unittest

from fake_upstream import FakeUpstream
from payments import charge


class Idempotency(unittest.TestCase):
    def test_lost_response_does_not_double_charge(self):
        up = FakeUpstream(drop_first_response=True)
        charge(up, "4242", 1000)
        self.assertEqual(len(up.charges), 1, "a retry after a lost response charged the card twice")

    def test_one_key_per_logical_charge(self):
        up = FakeUpstream(drop_first_response=True)
        charge(up, "4242", 1000)
        self.assertTrue(all(up.keys_seen), "every attempt must carry an idempotency key")
        self.assertEqual(len(set(up.keys_seen)), 1, "all attempts of one charge must reuse the same key")


if __name__ == "__main__":
    unittest.main()
