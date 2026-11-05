---
section: Added
---
- `ScEndpoint` reconnects, fails over to a second hub and reports its link
  like `BACnetClient`. In Rust, `ScEndpointBuilder::connect_hub_session`
  dials through the new `ScHubDialer`, shared with the SC client (dial logs
  now come from `bacnet_transport`); `EndpointSession::sc_link` reports it
  (#1946).
