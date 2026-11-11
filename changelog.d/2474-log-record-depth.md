---
section: Changed
---
- **Breaking (Rust API):** The log record encoders, and so `add_record` and
  log restore, refuse a record nested deeper than a ReadRange-ACK can carry
  and an opaque primitive over 1 MiB; `is_record_too_deep` tells the first
  refusal apart (#2474).
