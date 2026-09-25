import unittest

from durations import parse_duration


class HeldOut(unittest.TestCase):
    def test_valid(self):
        cases = {"2h": 7200, "1h0m5s": 3605, "10m": 600, "0s": 0, "1m1s": 61,
                 "  10m\n": 600, "\t3s ": 3, "100s": 100, "2h5s": 7205}
        for text, seconds in cases.items():
            with self.subTest(text=text):
                self.assertEqual(parse_duration(text), seconds)
                self.assertIs(type(parse_duration(text)), int)

    def test_invalid(self):
        for text in ["", "   ", "90", "h", "1d", "1H", "1h 30m", "30m1h", "1h1h",
                     "-5s", "+5s", "1.5h", "5 s", "1h30", "s5", "1h30m!"]:
            with self.subTest(text=text):
                with self.assertRaises(ValueError):
                    parse_duration(text)


if __name__ == "__main__":
    unittest.main()
