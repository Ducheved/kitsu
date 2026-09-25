+++
title = "A dead webhook endpoint holds a delivery worker forever"
scope = ["webhooks.py"]
checks = ["webhooks"]
+++
One customer's endpoint has been down since Friday, and a delivery worker
has been stuck in `deliver()` for it ever since. Make delivery give up.
