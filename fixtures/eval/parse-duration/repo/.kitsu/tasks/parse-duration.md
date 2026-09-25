+++
title = "Parse durations like 1h30m in the config"
scope = ["durations.py"]
checks = ["durations"]
+++
Config values like `timeout = "1h30m"` need to become seconds. Implement
`parse_duration(text) -> int` in `durations.py`:

- A duration is one or more `<digits><unit>` parts, with units `h`, `m`
  and `s`, each unit at most once and in that order: `2h`, `45s`,
  `1h30m`, `1h0m5s`. Digits are ASCII `0-9`.
- Leading and trailing whitespace is ignored. Whitespace inside is not
  allowed.
- Anything else raises `ValueError`: an empty string, a bare number
  (`90`), an unknown unit (`1d`, `1H`), a repeated unit or the wrong order
  (`30m1h`), a sign (`-5s`), a fraction (`1.5h`).
- `0s` is 0.
