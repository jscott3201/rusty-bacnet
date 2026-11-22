---
section: Added
---
- **Python API:** `RoutedTarget(..., arcnet=True)` marks the station's
  network as ARCNET; without it a call to a `RoutedTarget` whose address is
  `b"\xff"`, or three or six octets of `0xFF`, raises `BacnetError` before
  anything is sent or looked up (#2709).
