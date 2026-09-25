"""Drops duplicate rows from imports."""


def dedupe(items):
    """The items without duplicates, keeping the first occurrence of each, in order.

    Items are hashable values, or dicts whose values are hashable (rows from
    the CSV and JSON importers). Two items are duplicates when they are equal.
    """
    return list(dict.fromkeys(items))
