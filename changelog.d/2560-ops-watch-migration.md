---
section: Migration notes
---
- **bacnet-ops watch (Rust API, #2560):** `watch::Point` names its device
  by `DeviceAddress`: build it with `device.point("ai:1")` instead of an
  instance. `Why::Missing` now means no router to a remote address, a moved
  device gives its points up with `Why::Moved { to }`, and an application
  can call `client.watch(points)` instead of running a `Hub`.
  [Watching points](docs/rust-api.md#watching-points) has the table.
