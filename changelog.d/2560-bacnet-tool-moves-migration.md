---
section: Migration notes
---
- **bacnet-tool (Rust API, #2560):** depend on `bacnet-tool` and import
  `bacnet_tool::{safety, class, decode, bbmd}` for the old `bacnet_ops`
  paths; call `bacnet_tool::bbmd::read_bdt(&engine, bbmd)` (and `read_fdt`,
  `bip`, `registration_bbmd`) for the `Engine` methods.
  [Moved to bacnet-tool](docs/rust-api.md#moved-to-bacnet-tool) has the
  full table.
