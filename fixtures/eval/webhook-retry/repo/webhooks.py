"""Delivers events to customers' webhook endpoints."""

import time
import uuid


def deliver(transport, url, event, sleep=time.sleep):
    """POST `event` (a dict with an "id") to `url` and return the response.

    `transport.post(url, json=..., headers=...)` returns a response with a
    `.status`, or raises ConnectionError when the receiver can't be reached.
    """
    # Receivers go down sometimes; keep trying until it goes through.
    while True:
        try:
            resp = transport.post(url, json=event, headers={"Webhook-Id": str(uuid.uuid4())})
        except ConnectionError:
            sleep(1)
            continue
        if resp.status < 300:
            return resp
        sleep(1)
