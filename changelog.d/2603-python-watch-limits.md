---
section: Changed
---
- **Python API:** a watch's `lifetime` is at most 28800 s (`ValueError`
  past it), `Device.watch`'s keywords default to None (the client's), and
  the 256 limit counts distinct watch settings rather than live watches;
  new reasons `"lifetime-refused"` and `"poll-only"` (#2603).
