---
section: Migration notes
---
- **bacnet-ops Client (Rust API, #2560):** rename `Engine` to `Client` and
  drop its `Arc`; read through `client.device(target).await?` instead of
  passing an instance to each read; pass a `DeviceAddress` to the
  `RequestLimits` gates and `Reader`.
  [Engine becomes Client](docs/rust-api.md#engine-becomes-client) has the
  full table.
