---
section: Migration notes
---
- **bacnet-device-core feature (Rust API, #2210):** a no_std build with `alloc` alone refuses writes of an event parameter and of port permission or stage limit elements with WRITE_ACCESS_DENIED. A firmware that writes them names `features = ["alloc", "structured-writes"]`; `std` builds need nothing.
