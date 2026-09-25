"""Invoice lines."""


def line_total(unit_price, quantity, vat_rate):
    """Gross amount of one invoice line, in cents precision.

    unit_price and vat_rate are strings from the catalog ("19.99", "0.19").
    """
    return round(float(unit_price) * quantity * (1 + float(vat_rate)), 2)


def render_line(name, unit_price, quantity, vat_rate):
    return f"{quantity} x {name}: {line_total(unit_price, quantity, vat_rate)}"
