+++
title = "Webhook retries: bounded, backed off, and the same Webhook-Id every time"
state = "accepted"
scope = ["webhooks.py"]
rejected = [
  "A fresh Webhook-Id per attempt: receivers dedupe on it, so a retry after a lost 200 becomes a second order in the customer's system",
  "Retrying every 4xx: a 400 or a 410 will never succeed, and retrying it only hammers the customer",
  "Retrying until it goes through: one dead endpoint ties up a worker forever",
]
+++
At most 5 attempts, with exponential backoff between them: sleep 1, 2, 4
and 8 seconds. Retry on connection errors, 429 and 5xx. Any other status
is final and is returned to the caller as is, on the first attempt. After
the fifth attempt the caller gets the last response, or the last
connection error is raised.

The `Webhook-Id` header is the event's own `id`, so it is the same on
every attempt and on a manual redelivery from the admin screen. Receivers
dedupe on it.
