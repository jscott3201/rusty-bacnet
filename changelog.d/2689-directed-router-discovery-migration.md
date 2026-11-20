---
section: Migration notes
---
- **Breaking (Rust API).** Pass a destination MAC before the wait in `BACnetClient::discover_routers` and `OpsClient::discover_routers`. Pass `None` for the existing broadcast behavior (#2689).
