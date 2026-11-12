---
section: Fixed
---
- **Breaking (Rust API):** the device core's `Reporter` keeps reports owed an
  invoke ID apart from those owed outbox room and looks at them again only as
  IDs come back, which a host reports with `invoke_ids_released`;
  `notification_deadline` gives each confirmed notification's deadline (#2512).
