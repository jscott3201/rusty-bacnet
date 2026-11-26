---
section: Changed
---
- **Breaking (Rust API):** a watch's COV lifetime is at most 8 hours, a
  watch ends when its hub shuts down instead of reporting its points gone,
  and the client's watch hubs give their ranges back when their last watch
  ends (#2603).
