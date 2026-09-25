"""Durations in config files, like `timeout = "1h30m"`."""

import re

_FORMAT = re.compile(r"(?:([0-9]+)h)?(?:([0-9]+)m)?(?:([0-9]+)s)?")


def parse_duration(text):
    """The number of seconds in a duration string."""
    m = _FORMAT.fullmatch(text.strip())
    if not m or not any(m.groups()):
        raise ValueError(f"not a duration: {text!r}")
    h, mi, s = (int(g or 0) for g in m.groups())
    return h * 3600 + mi * 60 + s
