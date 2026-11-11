---
section: Changed
---
- **Breaking (Rust API):** `NotificationParameters` gains `Opaque { tag, data }`,
  and `LogDatum`, `LogValue`, `LogData` and `EventLogDatum` gain
  `Opaque(OpaqueChoice)`; `LogDatum` converts from a `LogValue` with
  `try_from` (#2474).
