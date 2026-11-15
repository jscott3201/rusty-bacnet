---
section: Changed
---
- **Breaking (MCP):** `bacnet_decode`'s `summary` renames `link` to
  `link_message`, the BVLC function or MS/TP frame type, and adds
  `original_source` and `parse_error`, which tells a header that fails to
  decode from a valid Error, Reject, Abort or NAK (#2614).
