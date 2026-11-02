---
section: Added
---
- **Breaking (Rust API):** `bacnet-device-core` has a `structured-writes` feature, which `std` turns on; a no_std build with `alloc` alone now refuses writes of a few structured datatypes it decodes itself, which drops about 20 KB of flash from the objects image (#2210).
