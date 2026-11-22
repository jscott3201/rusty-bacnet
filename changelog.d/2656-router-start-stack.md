---
section: Fixed
---
- Starting a `BACnetRouter` no longer builds its 150 KB of empty storage on
  the caller's stack: a release build asks the allocator for zeroed memory,
  which saves about 216 KiB of stack, and a debug build copies a constant,
  saving about 184 KiB (#2656).
