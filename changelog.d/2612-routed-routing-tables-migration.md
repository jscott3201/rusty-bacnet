---
section: Migration notes
---
- **Breaking (Rust API).** Pass `None` as the second argument to `NetworkLayer::send_routing_table_query` for a local query, or a destination station for a routed query. `BACnetClient::routing_table` accepts a `ToStation`, including existing link MACs (#2612).
