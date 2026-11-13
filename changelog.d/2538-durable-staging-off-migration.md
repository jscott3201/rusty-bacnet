---
section: Migration notes
---
- A build that drives `DurableWrites` on the built-in objects without `bacnet-objects`' `durable-writes` feature must enable it; the default `std` build already does (#2538).
