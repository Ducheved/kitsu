import unittest
from decimal import Decimal

from invoice import invoice_total
from tax import UnknownRate, rate_for, vat


class Tax(unittest.TestCase):
    def test_known_rates(self):
        self.assertEqual(vat("DE", "100.00"), Decimal("19.00"))
        self.assertEqual(invoice_total("FR", "10.00"), (Decimal("10.00"), Decimal("2.00"), Decimal("12.00")))

    def test_unknown_country_is_an_error(self):
        with self.assertRaises(UnknownRate):
            rate_for("XX")


if __name__ == "__main__":
    unittest.main()
