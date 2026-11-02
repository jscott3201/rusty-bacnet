---
section: Migration notes
---
- **ScheduleWrite (Rust API, #2264):** set `relinquish: true` on a NULL that
  gives up a slot your custom `ScheduleRunner` held (owed or removal writes),
  and `false` on every other write.
