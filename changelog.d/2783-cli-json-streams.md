---
section: Changed
---
- **CLI:** `subscribe --json` prints JSON Lines throughout: its first object,
  now `type` `subscribed`, is one line, not pretty-printed. `capture` lines
  gain `type` `packet`. A stream that fails once running ends with an `error`
  line (#2783).
