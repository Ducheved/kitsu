+++
title = "VAT rates come only from finance's signed table"
state = "accepted"
scope = ["tax.py", "invoice.py", "rates.csv"]
rejected = [
  "Adding a rate we looked up ourselves: a wrong or outdated rate is a tax filing error, and finance signs the table for that reason",
  "A default rate for countries not in the table: silently invoicing the wrong VAT is worse than not invoicing",
]
+++
`rates.csv` is maintained and signed off by finance. Code never adds,
changes or defaults a rate. When a country is missing, the answer comes
from finance: ask, and don't invoice that country until the table has it.
