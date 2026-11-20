---
section: Migration notes
---
- **Breaking (Rust API).** Pass a destination MAC before the wait in `BACnetClient::discover_routers` and `bacnet_ops::Client::discover_routers`. To broadcast, use `None` in these calls and in `who_is_router_to_network` calls that previously passed an explicit broadcast MAC (#2689).
