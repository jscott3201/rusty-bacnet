---
section: Changed
---
- **Rust API:** the target grammar's link refusals (`bacnet_ops::target`) name no CLI flag, so the Python binding and MCP show them as they are; the `bacnet` tool adds `--ipv6` or `--mstp` itself. `target::NEEDS_BIP6` and `NEEDS_MSTP` are the refusals' endings (#2561).
