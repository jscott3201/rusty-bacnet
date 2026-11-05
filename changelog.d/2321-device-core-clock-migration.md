---
section: Migration notes
---
- **Responder (Rust API, #2321):** `Responder::write` and `handle` take
  `&mut self`, since a time synchronization sets the clock it holds: hold
  the responder under the database's exclusive guard, or bind it `mut`.
  `classify` returns `Access::Write` for TimeSynchronization and
  UTCTimeSynchronization even with no clock, so a host that ran its own
  handler on `Access::None` routes them on `write`'s `Outcome::Unhandled`.
