+++
title = "Checkout shows prices from hours ago"
scope = ["cache.py"]
checks = ["cache"]
+++
After the price change at 9:00, checkout kept showing old prices until the
workers restarted around noon. The price cache has a 60 s TTL. `TTLCache`'s
docstring says how it should behave; `test_cache` shows one way it doesn't.
