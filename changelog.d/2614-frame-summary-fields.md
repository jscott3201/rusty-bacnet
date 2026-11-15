---
section: Changed
---
- **Breaking (Rust API):** `bacnet_ops::decode::frame::FrameSummary` renames
  `link` to `link_message`, adds `original_source`, a forwarded message's
  originating address, and `parse_error`, set only when a header fails to
  decode; `Link` adds `original_source`, and `Framing` is non-exhaustive
  (#2614).
