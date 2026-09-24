"""An upstream that can lose responses, like real ones do."""

from payments import Timeout


class FakeUpstream:
    def __init__(self, drop_first_response=False, always_timeout=False):
        self.charges = {}
        self.calls = 0
        self.keys_seen = []
        self.drop_first_response = drop_first_response
        self.always_timeout = always_timeout

    def post(self, path, card, amount, idempotency_key=None):
        self.calls += 1
        if self.calls > 50:
            raise RuntimeError("client is retrying forever")
        if self.always_timeout:
            raise Timeout()
        self.keys_seen.append(idempotency_key)
        key = idempotency_key or f"anon-{self.calls}"
        if key not in self.charges:
            self.charges[key] = (card, amount)
        if self.drop_first_response and self.calls == 1:
            # The charge happened. The response didn't make it back.
            raise Timeout()
        return {"id": key}
