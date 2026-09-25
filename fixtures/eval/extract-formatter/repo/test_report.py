import unittest

from report import csv_report, text_report


class Reports(unittest.TestCase):
    def test_text(self):
        self.assertEqual(
            text_report([("Rent", 120000), ("Refund", -2550)]),
            "Rent                        1,200.00\n"
            "Refund                       (25.50)\n"
            "Total                       1,174.50\n",
        )

    def test_csv(self):
        self.assertEqual(
            csv_report([("Coffee", 450), ("Refund", -99)]),
            "label,amount\nCoffee,4.50\nRefund,-0.99\n",
        )


if __name__ == "__main__":
    unittest.main()
