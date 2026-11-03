---
section: Changed
---
- **Breaking (Rust API):** Durable Trend Logs, Trend Log Multiples and Event
  Logs keep the reporting settings a peer writes, saved before the write is
  answered; a saved one wins over its setter until `clear_written_reporting`
  drops it (#1952).
