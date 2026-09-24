"""Charges a card through an upstream payment API."""

import uuid


class Timeout(Exception):
    pass


def charge(upstream, card, amount):
    # Upstream has been flaky, so keep trying until it goes through.
    while True:
        try:
            return upstream.post("/charges", card=card, amount=amount)
        except Timeout:
            continue
