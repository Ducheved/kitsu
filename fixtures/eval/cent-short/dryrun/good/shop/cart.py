"""The checkout page's running total."""

from decimal import Decimal

from shop.invoice import line_total


def cart_total(lines):
    """lines: [(unit_price, quantity, vat_rate)], strings as in the catalog.
    What the customer pays at checkout, which is what the invoice will say."""
    return sum((line_total(p, q, v) for p, q, v in lines), Decimal("0"))
