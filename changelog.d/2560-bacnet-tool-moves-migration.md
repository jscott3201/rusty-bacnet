---
section: Migration notes
---
- **bacnet-tool (Rust API, #2560):** depend on `bacnet-tool` and import
  `bacnet_tool::{safety, class, bbmd}` and `bacnet_tool::decode::tree` for
  the old `bacnet_ops` paths; frame summaries stay in `bacnet_ops::decode`.
  Call `bacnet_tool::bbmd::read_bdt(&client, bbmd)` (and `read_fdt`, `bip`,
  `registration_bbmd`) for the `Engine` methods.
  [Moved to bacnet-tool](docs/rust-api.md#moved-to-bacnet-tool) has the
  full table.
