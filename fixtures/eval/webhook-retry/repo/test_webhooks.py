import unittest

from fake_receiver import FakeReceiver
from webhooks import deliver

EVENT = {"id": "evt_123", "type": "order.paid"}


class Delivery(unittest.TestCase):
    def test_gives_up_on_a_dead_endpoint(self):
        rx = FakeReceiver(then="down")
        with self.assertRaises(ConnectionError):
            deliver(rx, "https://example.test/hook", EVENT, sleep=lambda s: None)
        self.assertLessEqual(len(rx.requests), 5, "at most 5 attempts")

    def test_normal_delivery(self):
        rx = FakeReceiver()
        self.assertEqual(deliver(rx, "https://example.test/hook", EVENT, sleep=lambda s: None).status, 200)
        self.assertEqual(len(rx.requests), 1)


if __name__ == "__main__":
    unittest.main()
