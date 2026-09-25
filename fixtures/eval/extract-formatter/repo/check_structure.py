"""The shape the refactor asks for: one formatting function, in money.py."""

import sys

import money

src = open("report.py").read()
problems = []
if not callable(getattr(money, "format_amount", None)):
    problems.append("money.py has no format_amount()")
if "divmod" in src or ":02d" in src:
    problems.append("report.py still formats amounts itself")
if "format_amount" not in src:
    problems.append("report.py doesn't use format_amount")
for p in problems:
    print(p)
sys.exit(1 if problems else 0)
