---
section: Changed
---
- **Breaking (Rust API):** the device core's `Reporter` keeps reports owed an
  invoke ID apart from those owed outbox room and looks at them again only as
  IDs come back, which a host reports with `invoke_ids_released` while the new
  `owes_invoke_id` holds; `notification_deadline` is new too (#2512).
