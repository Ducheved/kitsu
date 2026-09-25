"""Partial refunds from the support tool."""

from decimal import Decimal

from shop.money import cents


def refund_amount(paid, fraction):
    """`fraction` ("0.5") of the amount `paid` ("19.99"), to the cent."""
    return cents(Decimal(paid) * Decimal(fraction))
