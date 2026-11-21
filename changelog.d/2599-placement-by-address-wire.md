---
section: Changed
---
- **Wire:** Placement by address in bacnet-ops, the CLI and Python's `RoutedTarget` sends a Who-Is addressed to the one station, then one Who-Is to its network only if nothing answers, taking only that station's I-Am; one found the second way is asked on its network first from then on (#2599).
