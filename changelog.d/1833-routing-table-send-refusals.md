---
section: Fixed
---
- **Breaking (Rust and Python API):** `routing_table()` reports a link that
  refuses the query for its state or kind (a BACnet/SC link off its hub, a
  router-application client's) as `RoutingTableError::Send`, so Python
  raises `BacnetError` for both, replacing the `ValueError` noted above
  (#1833).
