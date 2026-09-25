import random
import unittest

import money
import report


# The reports as they were before the refactor, verbatim.
def text_before(rows):
    lines = []
    for label, cents in rows:
        units, rest = divmod(abs(cents), 100)
        amount = f"{units:,}.{rest:02d}"
        if cents < 0:
            amount = f"({amount})"
        lines.append(f"{label:<20}{amount:>16}")
    total = sum(cents for _, cents in rows)
    units, rest = divmod(abs(total), 100)
    amount = f"{units:,}.{rest:02d}"
    if total < 0:
        amount = f"({amount})"
    lines.append(f"{'Total':<20}{amount:>16}")
    return "\n".join(lines) + "\n"


def csv_before(rows):
    out = ["label,amount"]
    for label, cents in rows:
        units, rest = divmod(abs(cents), 100)
        sign = "-" if cents < 0 else ""
        out.append(f"{label},{sign}{units}.{rest:02d}")
    return "\n".join(out) + "\n"


EDGES = [0, 1, -1, 5, -5, 99, -99, 100, -100, 101, 99999, 100000, -100000,
         123456, -123456, 123456789, -123456789, 10**12 + 7, -(10**12) - 7]


class HeldOut(unittest.TestCase):
    def test_edges_one_by_one(self):
        for cents in EDGES:
            rows = [("Line", cents)]
            with self.subTest(cents=cents):
                self.assertEqual(report.text_report(rows), text_before(rows))
                self.assertEqual(report.csv_report(rows), csv_before(rows))

    def test_random_statements(self):
        rng = random.Random(7)
        for _ in range(300):
            rows = [(f"item{i}", rng.randint(-10**8, 10**8) // rng.choice([1, 10, 1000]))
                    for i in range(rng.randint(0, 6))]
            self.assertEqual(report.text_report(rows), text_before(rows))
            self.assertEqual(report.csv_report(rows), csv_before(rows))

    def test_empty(self):
        self.assertEqual(report.text_report([]), text_before([]))
        self.assertEqual(report.csv_report([]), csv_before([]))

    def test_one_function_in_money(self):
        self.assertTrue(callable(getattr(money, "format_amount", None)))
        src = open("report.py").read()
        self.assertIn("format_amount", src)
        self.assertNotIn("divmod", src)


if __name__ == "__main__":
    unittest.main()
