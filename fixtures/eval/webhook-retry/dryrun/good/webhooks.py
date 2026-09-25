"""Delivers events to customers' webhook endpoints."""

import time

MAX_ATTEMPTS = 5


def _retryable(status):
    return status == 429 or status >= 500


def deliver(transport, url, event, sleep=time.sleep):
    """POST `event` (a dict with an "id") to `url` and return the response.

    `transport.post(url, json=..., headers=...)` returns a response with a
    `.status`, or raises ConnectionError when the receiver can't be reached.
    """
    # Receivers dedupe on Webhook-Id: it is the event's id on every attempt.
    headers = {"Webhook-Id": event["id"]}
    for attempt in range(MAX_ATTEMPTS):
        last = attempt == MAX_ATTEMPTS - 1
        try:
            resp = transport.post(url, json=event, headers=headers)
        except ConnectionError:
            if last:
                raise
        else:
            if resp.status < 300 or not _retryable(resp.status) or last:
                return resp
        sleep(2**attempt)
