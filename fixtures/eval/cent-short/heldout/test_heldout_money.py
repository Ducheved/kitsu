import unittest
from decimal import Decimal

from shop.analytics import average_basket, conversion_rate
from shop.cart import cart_total
from shop.invoice import line_total, render_line
from shop.refunds import refund_amount


class HeldOut(unittest.TestCase):
    def test_invoice_lines(self):
        cases = [(("1.15", 1, "0.10"), "1.27"), (("1.25", 2, "0.19"), "2.98"),
                 (("2.05", 3, "0.10"), "6.77"), (("10.00", 1, "0.19"), "11.90"),
                 (("0.01", 1, "0"), "0.01")]
        for args, want in cases:
            with self.subTest(args=args):
                got = line_total(*args)
                self.assertIsInstance(got, Decimal)
                self.assertEqual(got, Decimal(want))

    def test_render_shows_cents(self):
        self.assertEqual(render_line("Tea", "1.50", 1, "0.19"), "1 x Tea: 1.79")

    def test_cart_is_the_sum_of_rounded_lines(self):
        # 2.975 + 1.265 = 4.24 rounded once; the invoice lines say 2.98 + 1.27 = 4.25.
        got = cart_total([("1.25", 2, "0.19"), ("1.15", 1, "0.10")])
        self.assertIsInstance(got, Decimal)
        self.assertEqual(got, Decimal("4.25"))
        self.assertEqual(cart_total([]), Decimal("0"))

    def test_refunds(self):
        for paid, fraction, want in [("19.99", "0.5", "10.00"), ("12.35", "0.5", "6.18"),
                                     ("9.99", "1", "9.99"), ("4.45", "0.5", "2.23")]:
            with self.subTest(paid=paid, fraction=fraction):
                got = refund_amount(paid, fraction)
                self.assertIsInstance(got, Decimal)
                self.assertEqual(got, Decimal(want))

    def test_dashboard_numbers_are_not_money(self):
        self.assertEqual(conversion_rate(1, 3), 33.3)
        self.assertIsInstance(conversion_rate(1, 3), float)
        self.assertEqual(conversion_rate(0, 0), 0.0)
        self.assertEqual(average_basket([10.4, 20.2]), 15)


if __name__ == "__main__":
    unittest.main()
