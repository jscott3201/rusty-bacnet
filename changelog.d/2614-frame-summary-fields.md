---
section: Changed
---
- **Breaking (Rust API):** `bacnet_ops::decode::frame::FrameSummary` adds
  `original_source`, the address a forwarded B/IP or B/IPv6 message carries,
  and `parse_error`, set only when a header fails to decode; `Link` adds
  `original_source`, and `Framing` is non-exhaustive (#2614).
