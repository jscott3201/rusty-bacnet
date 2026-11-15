---
section: Added
---
- **Python API:** `decode_frame(frame.octets, frame.link)` decodes an observed
  frame's headers through Rust into a `FrameSummary`, telling a parse failure
  from a protocol refusal and keeping a forwarded message's originating
  address and the routed addresses (#2614).
