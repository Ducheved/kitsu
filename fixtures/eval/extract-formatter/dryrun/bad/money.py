"""Formatting amounts held as integer cents."""


def format_amount(cents, parens=False):
    amount = f"{abs(cents) / 100:,.2f}"
    if cents < 0:
        return f"({amount})" if parens else f"-{amount}"
    return amount
