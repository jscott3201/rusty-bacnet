---
section: Changed
---
- **Breaking (Rust API):** `bacnet-server`'s WriteProperty path now runs on
  `bacnet-device-core`, whose `write` module holds it, and firmware can run
  WriteProperty with its `Responder` (#2178).
