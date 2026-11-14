---
section: Migration notes
---
- **ops discovery (Rust API, #2560):** write `client.discover(scope, range,
  wait)` as `client.discover(Discover::from(scope).range(low..=high).wait(wait))`,
  and `client.who_is(scope, range)` as `discover` or `scope.who_is(client.low_level(),
  range)`. `Scope::Directed` takes a `DeviceAddress`: build it from
  `client.station_mac(text)`.
