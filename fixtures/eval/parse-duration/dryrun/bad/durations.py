"""Durations in config files, like `timeout = "1h30m"`."""

import re

UNITS = {"h": 3600, "m": 60, "s": 1}


def parse_duration(text):
    """The number of seconds in a duration string."""
    parts = re.findall(r"(\d+)([hms])", text)
    if not parts:
        raise ValueError(f"not a duration: {text!r}")
    return sum(int(n) * UNITS[u] for n, u in parts)
