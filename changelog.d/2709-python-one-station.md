---
section: Added
---
- **Python API:** `RoutedTarget(..., one_station=True)` marks the address
  one station; without it a call to a `RoutedTarget` whose address is a
  broadcast or group on its network, such as `b"\xff"`, raises `BacnetError`
  before anything is sent or looked up (#2709, #2738).
