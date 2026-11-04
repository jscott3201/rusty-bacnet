---
section: Migration notes
---
- **Responder (Rust API, #2321):** `bacnet_device_core::responder::Responder`'s
  `write` and `handle` take `&mut self`, since a time synchronization sets
  the clock it holds: bind the responder `mut`.
