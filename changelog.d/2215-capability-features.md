---
section: Added
---
- **Breaking (Rust API):** `bacnet-objects` has `event-reporting` and `durable-writes` features, which `std` turns on; a no_std build with `alloc` alone now has both off, which drops about 4.4 to 6 KB of flash from the objects image (#2215).
