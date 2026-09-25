import unittest

from fake_receiver import FakeReceiver
from webhooks import deliver

URL = "https://example.test/hook"
EVENT = {"id": "evt_42", "type": "order.paid"}


def run(rx):
    slept = []
    try:
        result = deliver(rx, URL, dict(EVENT), sleep=slept.append)
    except ConnectionError as e:
        result = e
    return result, slept


class HeldOut(unittest.TestCase):
    def test_same_webhook_id_on_every_attempt_and_it_is_the_event_id(self):
        rx = FakeReceiver(["down", 503, "down"], then=200)
        result, _ = run(rx)
        self.assertEqual(result.status, 200)
        ids = [r["headers"].get("Webhook-Id") for r in rx.requests]
        self.assertEqual(ids, ["evt_42"] * 4)

    def test_backoff_between_five_attempts(self):
        rx = FakeReceiver(then="down")
        result, slept = run(rx)
        self.assertIsInstance(result, ConnectionError)
        self.assertEqual(len(rx.requests), 5)
        self.assertEqual(slept, [1, 2, 4, 8])

    def test_last_response_after_five_5xx(self):
        rx = FakeReceiver(then=503)
        result, _ = run(rx)
        self.assertEqual((result.status, len(rx.requests)), (503, 5))

    def test_429_is_retried(self):
        rx = FakeReceiver([429], then=200)
        result, slept = run(rx)
        self.assertEqual((result.status, len(rx.requests), slept), (200, 2, [1]))

    def test_other_4xx_is_final(self):
        for status in (400, 404, 410):
            with self.subTest(status=status):
                rx = FakeReceiver([status], then=200)
                result, slept = run(rx)
                self.assertEqual((result.status, len(rx.requests), slept), (status, 1, []))

    def test_success_needs_no_sleep(self):
        rx = FakeReceiver()
        result, slept = run(rx)
        self.assertEqual((result.status, slept), (200, []))


if __name__ == "__main__":
    unittest.main()
