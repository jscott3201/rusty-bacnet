---
section: Migration notes
---
- **Breaking (Rust API).** Import `Field`, `decode_tree`, `decode_npdu_tree` and `decode_apdu_tree` from `bacnet_ops::decode::tree`. Update dependencies that used `bacnet_tool::decode::tree` (#2633).
