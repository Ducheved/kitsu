"""Partial refunds from the support tool."""


def refund_amount(paid, fraction):
    """`fraction` ("0.5") of the amount `paid` ("19.99"), to the cent."""
    return round(float(paid) * float(fraction), 2)
