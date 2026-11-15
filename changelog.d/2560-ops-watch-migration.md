---
section: Migration notes
---
- **bacnet-ops watch (Rust API, #2560):** import `Hub`, `Lease`,
  `Snapshot`, `WatchOptions` and the other hub types from `watch::hub`;
  the snapshot's `State` and `Source` are `watch::hub::Phase` and `Source`.
  Build a `watch::Point` with `device.point("ai:1")`: it names a
  `DeviceAddress`, not an instance. Match `Why` and `WatchError` with a
  wildcard arm. Applications call `client.watch(points)`.
  [Watching points](docs/rust-api.md#watching-points) has the table.
