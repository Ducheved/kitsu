"""VAT on invoices, from finance's table (rates.csv)."""

import csv
import os
from decimal import ROUND_HALF_UP, Decimal


class UnknownRate(KeyError):
    """No VAT rate for this country in finance's table."""


def _load():
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "rates.csv")
    with open(path, newline="") as f:
        rows = [r for r in csv.reader(f) if r and not r[0].startswith("#")]
    return {country: Decimal(rate) for country, rate in rows[1:]}


RATES = _load()


def rate_for(country):
    try:
        return RATES[country]
    except KeyError:
        raise UnknownRate(country) from None


def vat(country, net):
    """VAT on a net amount ("100.00"), to the cent."""
    return (Decimal(net) * rate_for(country)).quantize(Decimal("0.01"), ROUND_HALF_UP)
