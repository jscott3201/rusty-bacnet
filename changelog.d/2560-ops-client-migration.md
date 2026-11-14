---
section: Migration notes
---
- **bacnet-ops Client (Rust API, #2560):** rename `Engine` to `Client`,
  the `bacnet_ops::engine` paths to `bacnet_ops::client`, and drop the
  `Arc`; read through `client.device(target).await?` instead of passing an
  instance to each read; pass a `DeviceAddress` to the `RequestLimits`
  gates and `Reader`.
  [Engine becomes Client](docs/rust-api.md#engine-becomes-client) has the
  full table.
