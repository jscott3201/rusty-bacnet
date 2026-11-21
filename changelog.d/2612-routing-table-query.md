---
section: Added
---
- **Python API:** a read-only routing-table query, `BACnetClient.routing_table(router)`, with `bacnet routing-table` and `BACnetClient::routing_table` in Rust; it sends Initialize-Routing-Table with no ports, so it cannot update a table, and only to one router's own address (#2612).
