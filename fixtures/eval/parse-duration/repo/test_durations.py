import unittest

from config import load
from durations import parse_duration


class Durations(unittest.TestCase):
    def test_hours_and_minutes(self):
        self.assertEqual(parse_duration("1h30m"), 5400)

    def test_seconds(self):
        self.assertEqual(parse_duration("45s"), 45)

    def test_unknown_unit(self):
        with self.assertRaises(ValueError):
            parse_duration("1d")

    def test_config_defaults(self):
        self.assertEqual(load({}), {"timeout": 30, "idle": 300})


if __name__ == "__main__":
    unittest.main()
