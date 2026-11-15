---
section: Changed
---
- **Rust API:** `BACnetClient::who_has_replies` keeps one answer per device,
  object and source address, so two devices sharing an instance both come
  back, and stops at `MAX_I_HAVE_REPLIES` (4096); `collect_answers` is that
  collector for a caller sending its own Who-Has (#2596).
