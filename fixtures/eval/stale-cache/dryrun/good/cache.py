"""A small in-process cache for price lookups."""

import time
from collections import OrderedDict


class TTLCache:
    """At most `maxsize` entries, each valid for `ttl` seconds after it was set.

    get() returns the value, or `default` if the key is missing or expired.
    get() of a live entry makes it the most recently used, and so does set().
    When a set() would go over maxsize, expired entries are dropped first,
    then the least recently used. len() counts live entries only.
    """

    def __init__(self, maxsize, ttl, clock=time.monotonic):
        self.maxsize = maxsize
        self.ttl = ttl
        self.clock = clock
        self._data = OrderedDict()  # key -> (expires_at, value), oldest use first

    def _drop_expired(self):
        now = self.clock()
        for key in [k for k, (exp, _) in self._data.items() if exp <= now]:
            del self._data[key]

    def set(self, key, value):
        self._data[key] = (self.clock() + self.ttl, value)
        self._data.move_to_end(key)
        if len(self._data) > self.maxsize:
            self._drop_expired()
        while len(self._data) > self.maxsize:
            self._data.popitem(last=False)

    def get(self, key, default=None):
        item = self._data.get(key)
        if item is None:
            return default
        expires_at, value = item
        if expires_at <= self.clock():
            del self._data[key]
            return default
        self._data.move_to_end(key)
        return value

    def __len__(self):
        self._drop_expired()
        return len(self._data)
