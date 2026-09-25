+++
title = "One place for amount formatting"
scope = ["report.py", "money.py"]
checks = ["report", "structure"]
+++
The divmod-and-format code is copied three times in `report.py`, and the
PDF statement coming next would make it four. Move it into a new
`money.py` as one function, `format_amount`, with whatever parameters the
two styles need, and use it from both reports.

This is a refactor: both reports' output must stay byte-for-byte the same.
