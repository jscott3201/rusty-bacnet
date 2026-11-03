---
section: Fixed
---
- `bacnet` prints a fatal error as its message, not its Debug form: no quotes
  or doubled backslashes, and `transport error: Can't assign requested address
  (os error 49)` instead of `Transport(Os { code: 49, … })`; exit code 1 is
  unchanged (#2325).
