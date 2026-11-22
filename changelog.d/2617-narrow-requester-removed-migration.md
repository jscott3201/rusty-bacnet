---
section: Migration notes
---
- **Endpoint client (Rust API, #2617):** keep one `session.client()?`. For
  `EndpointApduDestination::Routed` pass `Station::routed(router, network,
  mac)` or `Station::remote(network, mac)`, for `RoutedViaLocalBroadcast`
  `Station::routed_via_local_broadcast(network, mac)`, and for
  `data_attributes` give `client_with` `ClientOptions::with_data_attributes`
  (#2723) ([table](docs/rust-api.md#the-narrow-client-role-is-gone)).
