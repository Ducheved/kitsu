"""Invoice totals for a customer."""

from decimal import Decimal

from tax import vat


def invoice_total(country, net):
    """(net, vat, gross) as Decimals for an invoice in `country`."""
    n = Decimal(net)
    v = vat(country, net)
    return n, v, n + v
