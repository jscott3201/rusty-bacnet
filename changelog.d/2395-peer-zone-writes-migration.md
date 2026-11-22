---
section: Migration notes
---
- **Device zone (Rust API, #2395):** `TagsSnapshot` gains `zone`
  (`WrittenZone`) and `ServerConfig` gains `clock_zone_writable`; exhaustive
  struct literals set both. A Device file holding a written half is
  `RBNTAG03`, which older builds refuse.
