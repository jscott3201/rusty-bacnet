---
section: Fixed
---
- `build_database_with_extra` refuses an extra object whose identifier is
  already taken by the Device, a declared Network Port or another extra,
  instead of silently replacing that object (#2250).
