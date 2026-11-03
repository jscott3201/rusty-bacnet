---
section: Fixed
---
- `bacnet` prints a fatal error as plain text on stderr (`Error: ` and the
  message), not quoted with escapes, so Windows paths no longer show doubled
  backslashes; the exit code stays 1 (#2325).
