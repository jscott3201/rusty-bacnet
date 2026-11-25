---
section: Fixed
---
- **Breaking (Rust and Python API):** `routing_table()` reports a link that
  refuses the query for its state, configuration, platform or a limit (a
  BACnet/SC link not connected to its hub, say) as `RoutingTableError::Send`,
  which Python raises as `BacnetError`, not `ValueError` (#1833).
