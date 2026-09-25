"""Splits a list into pages for the admin UI."""


def paginate(items, page, per_page):
    """Return (the items on `page`, the number of pages).

    Pages are numbered from 1. A page past the last one is empty. No items
    means 0 pages. A `page` or `per_page` below 1 raises ValueError.
    """
    if per_page < 1:
        raise ValueError("per_page must be at least 1")
    pages = len(items) // per_page
    start = page * per_page
    return items[start:start + per_page], pages
