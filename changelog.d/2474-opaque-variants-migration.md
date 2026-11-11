---
section: Migration notes
---
- **Unknown choices (Rust API, #2474):** a `match` on `NotificationParameters`,
  `LogDatum`, `LogValue`, `LogData` or `EventLogDatum` needs an `Opaque` arm:
  a choice a later revision adds, its octets kept as received, which the
  encoders write back unchanged. Replace `LogDatum::from(value)` or
  `value.into()` with `LogDatum::try_from(value)`, whose error is the opaque
  member value, which has no Trend Log datum.
