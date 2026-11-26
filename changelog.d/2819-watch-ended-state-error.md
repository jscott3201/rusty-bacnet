---
section: Changed
---
- **Python API:** `Watch.add()` and `Watch.remove()` on a watch that has
  ended raise `BacnetStateError` (`state == "closed"`), message unchanged
  (#2819). `except RuntimeError` still catches it; `except BacnetError` now
  does too.
