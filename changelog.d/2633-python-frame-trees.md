---
section: Added
---
- **Python API:** `decode_frame_tree(octets, link)` decodes a frame offline
  into frozen `FieldNode` trees: named service fields, each with its span in
  the input, and partial fields with diagnostics for malformed input
  (#2633).
