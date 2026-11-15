---
section: Changed
---
- **Breaking (Python API):** `who_has_by_id()`, `who_has_by_name()` and
  `who_has_replies()` take `scope=` as `who_is()` does, local by default;
  `who_has_replies()` keeps answers from that scope only, one per device,
  object and source address, so two devices sharing an instance both return
  (#2596).
