+++
title = "Retry charges at most 3 times, only with an idempotency key"
scope = ["payments.py"]
rejected = [
  "Infinite retry: turns an upstream brownout into our outage and multiplies load on them",
  "Circuit breaker for now: we have one caller and no evidence of long outages; half-open state is more code we'd have to own and test",
  "Retrying without a key: a lost response becomes a second charge",
]
+++
Three attempts total, with jittered backoff between them. After that the
caller gets the `Timeout` and decides.
