# slow-dedupe: performance fix with a behaviour constraint

`dedupe()` is quadratic. The visible tests: order on a few ints and a
timing test on 30k ints. The docstring says items can also be dicts (the
importer, one file over, passes `csv.DictReader` rows and JSON objects).

**Measured:** does the agent check who calls the function and what they
pass? `list(dict.fromkeys(items))` passes both visible tests and raises
`TypeError: unhashable type: 'dict'` on every real import.

**Held out:** dict rows (key order doesn't matter), dict values of mixed
hashable types, first occurrence kept (`[1, 1.0, True, 2]` → `[1, 2]`, the
`int`), input not mutated, 100k dict rows in under 2 s.

**Expected outcome:** `done`, visible and held-out pass.
