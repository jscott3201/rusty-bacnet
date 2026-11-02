---
section: Migration notes
---
- **bacnet-objects features (Rust API, #2215):** a no_std build with `alloc` alone now has `event-reporting` and `durable-writes` off, so its objects lend no event reporting and stage no durable writes. A no_std host that drives either adds the feature; `std` builds need nothing.
