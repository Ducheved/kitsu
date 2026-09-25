"""Money is Decimal, rounded half up to the cent (decision money-is-decimal)."""

from decimal import ROUND_HALF_UP, Decimal

CENT = Decimal("0.01")


def cents(amount):
    return Decimal(amount).quantize(CENT, rounding=ROUND_HALF_UP)
