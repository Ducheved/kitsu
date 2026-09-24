+++
title = "One idempotency key per logical charge, reused by every retry"
state = "accepted"
scope = ["payments.py"]
+++
The upstream can charge the card and then lose the response. The only thing
that makes a retry safe is sending the same `idempotency_key` again, so the
key has to be created once per logical charge, before the first attempt.
A key per attempt looks fine in every happy-path test and double-charges
people in production.

Enforced by `idempotency`; comes from decision `retry-budget`.
