---
section: Added
---
- **Python API:** `Device.inventory()` reads a device's points as `Point` records (names, units, state texts, commandable, values) into an `Inventory` that keeps a partial read, and `Device.points()` hands the same records out as an async iterator, a batch at a time (#2561).
