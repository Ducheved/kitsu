"""Prices from the pricing service, cached for a minute."""

from cache import TTLCache

_cache = TTLCache(maxsize=1000, ttl=60)


def price(sku, fetch):
    """The current price of `sku`; `fetch(sku)` asks the pricing service."""
    hit = _cache.get(sku)
    if hit is None:
        hit = fetch(sku)
        _cache.set(sku, hit)
    return hit
