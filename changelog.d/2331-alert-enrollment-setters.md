---
section: Changed
---
- **Breaking (Rust API):** `AlertEnrollmentObject`'s `event_enable`,
  `notification_class` and `event_detection_enable` fields are private; set
  them through its setters, which a saved write wins over (#2331).
