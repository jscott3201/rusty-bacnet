---
section: Added
---
- `bacnet-ops`: `Device::inventory` lists a device's objects as `PointRecord`s (name, units, state
  texts, commandability, values when asked), read in batches and written as JSON or CSV; the CLI
  gains `bacnet inventory <target> [--types] [--values] --format json|csv|table` (#2560).
