---
section: Fixed
---
- `stop()` now waits, for at most 5 s, for a local write dropped while the database guard was taken to let go of its stage, so storage holds what each object serves when `stop()` returns, unless the application still holds the database after those 5 s (#2495).
