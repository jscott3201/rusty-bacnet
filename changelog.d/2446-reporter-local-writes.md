---
section: Added
---
- bacnet-device-core's `Reporter` makes the application's own changes and
  reports them: `update`, `set_reliability`, `command`, `relinquish` and
  `apply_local`, so firmware no longer calls `changed` by hand after each
  one (#2446).
