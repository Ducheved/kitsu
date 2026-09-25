"""Formatting amounts held as integer cents."""


def format_amount(cents, *, grouping, parens):
    """`cents` as units.cents. `grouping` adds thousands separators;
    `parens` shows negatives as (1.00) instead of -1.00."""
    units, rest = divmod(abs(cents), 100)
    amount = f"{units:,}.{rest:02d}" if grouping else f"{units}.{rest:02d}"
    if cents < 0:
        return f"({amount})" if parens else f"-{amount}"
    return amount
