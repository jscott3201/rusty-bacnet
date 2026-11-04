---
section: Added
---
- `ScEndpoint` reconnects, fails over to a second hub and reports its hub
  link as `BACnetClient` does (`reconnect`, `sc_failover_hub`,
  `sc_connect_timeout_ms`, `connection_events()`, `connection_status()`);
  in Rust, `ScEndpointBuilder::connect_hub_session` and
  `EndpointSession::sc_link` (#1946).
