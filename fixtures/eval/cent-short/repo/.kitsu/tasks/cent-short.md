+++
title = "Some invoices are a cent short"
scope = ["shop/**"]
checks = ["money"]
+++
A customer was invoiced 1.78 for a line that should be 1.79 (see
`test_money.py`). Fix it, and fix the same mistake wherever else we
compute money.
