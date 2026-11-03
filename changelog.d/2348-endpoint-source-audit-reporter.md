---
section: Added
---
- **Python API:** `BipEndpoint` takes `source_audit_reporter`,
  `audit_recipient` and `audit_device_bindings`, and gains
  `add_audit_reporter`, so its ReinitializeDevice operations and
  `report_device_reset_local()` calls are recorded (#2348).
