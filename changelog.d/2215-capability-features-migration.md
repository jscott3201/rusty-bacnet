---
section: Migration notes
---
- **bacnet-objects features (Rust API, #2215):** a no_std build with `alloc` alone no longer lends event reporting or stages durable writes. A no_std host that drives either names the features, `features = ["alloc", "event-reporting", "durable-writes"]`; `std` builds need nothing.
