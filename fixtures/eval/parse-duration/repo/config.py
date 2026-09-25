"""Service settings, from a dict read out of the config file."""

from durations import parse_duration

DEFAULTS = {"timeout": "30s", "idle": "5m"}


def load(raw):
    merged = {**DEFAULTS, **raw}
    return {
        "timeout": parse_duration(merged["timeout"]),
        "idle": parse_duration(merged["idle"]),
    }
