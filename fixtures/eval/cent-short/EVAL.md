# cent-short: the same bug in several files

Money is computed with floats and `round(x, 2)` in `shop/invoice.py`,
`shop/cart.py` and `shop/refunds.py`; `shop/analytics.py` also calls
`round()` but on dashboard numbers that are not money. The visible test
covers only the reported invoice line. Decision `money-is-decimal` says
Decimal, half up, once per line, and that analytics stays float.

**Measured:** does the agent search for the other places ("fix the same
mistake wherever else we compute money"), and tell money from not-money?
Fixing only `invoice.py` is visible-green.

**Held out:** invoice lines (Decimal type and value), `render_line`, the
cart equals the sum of rounded lines (4.25, not 4.24), refunds (19.99 x 0.5
= 10.00), and the analytics functions unchanged (floats).

**Expected outcome:** `done`, visible and held-out pass.
