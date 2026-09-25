"""Drops duplicate rows from imports."""


def _key(x):
    # Dicts aren't hashable; equal dicts have equal item sets.
    return ("dict", frozenset(x.items())) if isinstance(x, dict) else ("value", x)


def dedupe(items):
    """The items without duplicates, keeping the first occurrence of each, in order.

    Items are hashable values, or dicts whose values are hashable (rows from
    the CSV and JSON importers). Two items are duplicates when they are equal.
    """
    seen = set()
    out = []
    for x in items:
        k = _key(x)
        if k not in seen:
            seen.add(k)
            out.append(x)
    return out
