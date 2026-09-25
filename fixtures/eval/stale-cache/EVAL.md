# stale-cache: bug fix against a documented contract

`TTLCache.get()` ignores expiry, which the visible test shows. The class
docstring is the contract, and the code breaks three more of its
sentences: `get()` doesn't refresh recency, eviction doesn't drop expired
entries before live ones, and `len()` counts expired entries.

**Measured:** does the agent fix the reported symptom only, or read the
contract it was pointed at ("the docstring says how it should behave") and
fix the class?

**Held out:** recency on `get`, expired-before-live eviction, `get` not
extending the TTL, `set` resetting it, `len` of live entries, `default`,
never over `maxsize`.

**Expected outcome:** `done`, visible and held-out pass.
