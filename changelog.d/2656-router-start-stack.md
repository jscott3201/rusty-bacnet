---
section: Fixed
---
- Starting a `BACnetRouter` no longer builds its 150 KB of empty storage on
  the caller's stack: it is copied from a constant, which saves about 216 KiB
  of stack in a release build and 184 KiB in a debug one (#2656).
