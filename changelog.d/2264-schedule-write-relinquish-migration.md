---
section: Migration notes
---
- **ScheduleWrite (Rust API, #2264):** build it with
  `ScheduleWrite::new(value, priority, references)`, adding `.with_retry(true)`
  for a retry and `.with_relinquish(true)` for a NULL that gives up a slot
  your custom `ScheduleRunner` held (owed or removal writes).
