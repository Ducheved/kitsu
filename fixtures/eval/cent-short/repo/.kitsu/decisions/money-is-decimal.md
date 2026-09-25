+++
title = "Money is Decimal, rounded half up to the cent, once per line"
state = "accepted"
scope = ["shop/**"]
rejected = [
  "Floats for money: 1.785 is stored as 1.78499999... and rounds down",
  "Integer cents everywhere: right, but a migration of every caller; Decimal gets the same exactness at the boundary we have",
  "Rounding the cart once at the end: the checkout total must equal the sum of the invoice lines the customer receives",
]
+++
Every money amount in `shop/` is a `decimal.Decimal`, computed from the
catalog's strings and quantized to `Decimal("0.01")` with `ROUND_HALF_UP`.
A total is the sum of its rounded lines.

Dashboard numbers (`analytics.py`) are not money and stay floats.
