import unittest
from decimal import Decimal

from shop.invoice import line_total


class Money(unittest.TestCase):
    def test_the_reported_invoice_line(self):
        # 1.50 + 19% VAT = 1.785, which is 1.79. The customer was invoiced 1.78.
        self.assertEqual(line_total("1.50", 1, "0.19"), Decimal("1.79"))


if __name__ == "__main__":
    unittest.main()
