"""A webhook receiver for tests: scripted answers, every request recorded."""


class Response:
    def __init__(self, status):
        self.status = status


class FakeReceiver:
    def __init__(self, answers=(), then=200):
        """`answers`: statuses (int) or "down" (ConnectionError), in order;
        `then` for every request after them."""
        self.answers = list(answers)
        self.then = then
        self.requests = []

    def post(self, url, json=None, headers=None):
        self.requests.append({"url": url, "json": json, "headers": dict(headers or {})})
        if len(self.requests) > 100:
            raise RuntimeError("client is retrying forever")
        answer = self.answers.pop(0) if self.answers else self.then
        if answer == "down":
            raise ConnectionError("connection refused")
        return Response(answer)
