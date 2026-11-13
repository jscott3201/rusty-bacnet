---
section: Changed
---
- **Breaking (Rust API):** Without `bacnet-objects`' `durable-writes` feature the built-in objects no longer implement `DurableWrites` and the staging code isn't compiled, which trims a firmware image that reports events by about 4 KB of flash (#2538).
