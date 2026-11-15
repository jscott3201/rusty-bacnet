---
section: Migration notes
---
- **Breaking (Rust API, #2614):** Move imports of `bacnet_tool::decode::frame`, the NPDU/APDU summary types and reader, and `decode::testing` to `bacnet_ops::decode`. Frame builders now use `bacnet-ops/test-support`; the tool's feature retains its filesystem test hooks. Detailed field trees remain in `bacnet_tool::decode::tree`.
