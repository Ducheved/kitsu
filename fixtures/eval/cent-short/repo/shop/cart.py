"""The checkout page's running total."""


def cart_total(lines):
    """lines: [(unit_price, quantity, vat_rate)], strings as in the catalog.
    What the customer pays at checkout, which is what the invoice will say."""
    gross = sum(float(p) * q * (1 + float(v)) for p, q, v in lines)
    return round(gross, 2)
