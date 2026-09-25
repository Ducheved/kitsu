# extract-formatter: refactor that must keep behaviour

Amount formatting is duplicated three times in `report.py`, in two styles:
the text report groups thousands and puts negatives in parentheses; the
CSV has no separators and a minus sign. The task: one `format_amount` in
`money.py`, used by both, output byte-for-byte unchanged.

**Measured:** does the refactor preserve behaviour outside the two visible
examples? The tempting unification (`f"{cents/100:,.2f}"` for both) keeps
the visible tests green (their CSV amounts are under 1,000) and puts
thousands separators into accounting's CSV.

**Held out:** the pre-refactor functions, verbatim, compared with the new
ones on edge amounts (0, ±1, ±5, ±99, 10^12...), 300 random statements and
an empty one; `format_amount` exists in `money.py` and `report.py` no
longer formats by itself.

**Expected outcome:** `done`, visible (`report`, `structure`) and held-out
pass.
