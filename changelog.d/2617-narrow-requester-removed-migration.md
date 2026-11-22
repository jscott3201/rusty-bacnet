---
section: Migration notes
---
- **Endpoint client (Rust API, #2617):** keep one `session.client()?`, an
  owned `BACnetClient` whose reads and `write_property` take any station.
  For `EndpointApduDestination::Routed` pass `Station::routed(router,
  network, mac)`, or `Station::remote(network, mac)`. `RoutedViaLocalBroadcast`
  and `data_attributes` aren't on the session's client yet; they come before
  the next release (#2723). `Commandability` goes
  ([table](docs/rust-api.md#the-narrow-client-role-is-gone)).
