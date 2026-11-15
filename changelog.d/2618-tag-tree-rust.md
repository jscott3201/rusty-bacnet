---
section: Added
---
- **Rust API:** `bacnet_encoding::tags::tree` reads any value as its tags
  without its datatype: `TagReader` with no allocator, and `decode_tag_tree`
  with `alloc`, bounded in depth and size, its faults typed (#2618).
