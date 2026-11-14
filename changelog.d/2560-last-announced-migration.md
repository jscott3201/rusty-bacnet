---
section: Migration notes
---
- **Discovery rows (Rust API, #2560):** a `DiscoveredDevice` built as a
  struct literal sets `last_announced`: when the device last sent an I-Am,
  or `None` for a row registered by hand.
