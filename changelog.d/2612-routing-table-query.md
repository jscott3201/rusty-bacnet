---
section: Added
---
- **Python API:** a read-only routing-table query, `BACnetClient.routing_table(router)`, with `bacnet routing-table` and `BACnetClient::routing_table` in Rust; it sends Initialize-Routing-Table with no ports, so it cannot update a table. `SENDABLE_NETWORK_MESSAGES` now has three entries (#2612).
