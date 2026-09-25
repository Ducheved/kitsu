"""Numbers for the internal dashboard."""


def conversion_rate(orders, visits):
    """Percent of visits that ended in an order, one decimal, for a chart."""
    return round(100 * orders / visits, 1) if visits else 0.0


def average_basket(totals):
    """Mean of order totals (floats are fine: the chart shows whole units)."""
    return round(sum(totals) / len(totals)) if totals else 0
