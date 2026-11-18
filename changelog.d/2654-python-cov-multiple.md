---
section: Changed
---
- **Breaking (Python API):** `cov_notifications()` also yields
  `CovNotificationMultiple`, with or without a `subscribe_cov_property_multiple`
  subscription (an unsubscribed broadcast); both classes gain `kind`,
  `"object"` or `"multiple"` (#2654).
