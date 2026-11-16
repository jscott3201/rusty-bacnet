---
section: Added
---
- **Python API:** `cov_notifications()` also yields `CovNotificationMultiple`,
  the decoded objects, values and times of change a
  `subscribe_cov_property_multiple` subscription reports; both classes gain
  `kind`, `"object"` or `"multiple"` (#2654).
