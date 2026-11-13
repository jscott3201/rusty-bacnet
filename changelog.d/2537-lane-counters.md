---
section: Added
---
- **Python API:** `event_notification_counters()`, and Rust's
  `EventNotificationCounters`, count notifications dropped at a full send
  lane (`lane_full`, `lanes_entries_full`, `lanes_full`) and COV fanouts that
  stopped waiting for their event notifications (`cov_wait_expired`) (#2537).
