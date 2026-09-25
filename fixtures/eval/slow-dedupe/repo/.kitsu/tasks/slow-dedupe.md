+++
title = "The nightly import spends most of an hour in dedupe()"
scope = ["dedupe.py"]
checks = ["dedupe"]
+++
The nightly import of about 100k rows spends most of an hour in
`dedupe()`. Make it fast. `test_dedupe` has a timing test.
