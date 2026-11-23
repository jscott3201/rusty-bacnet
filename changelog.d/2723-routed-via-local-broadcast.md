---
section: Added
---
- **Breaking (Rust API):** `Station::RoutedViaLocalBroadcast`
  (`Station::routed_via_local_broadcast`) sends a confirmed request to a
  station on another network by one broadcast on the link while no router
  to it is known, as the endpoint's removed requester did (#2723).
