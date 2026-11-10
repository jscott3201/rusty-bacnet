---
section: Migration notes
---
- **COV-multiple sizing helpers (Rust API, #2320):**
  `bacnet_device_core::cov::timed::ITEM_FRAMING`, `value_len` and
  `request_header` are private. Split a report with
  `cov::multiple_chunks::split`; `request_limit` and `untimed_octets`
  there give the sizes a host needs.
