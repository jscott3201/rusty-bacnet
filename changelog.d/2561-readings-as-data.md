---
section: Added
---
- **Python API:** `Device.read_many` returns `Readings`: a refused property is its `Reading.error` and a device going silent is `stopped`, neither raised. `Value` gains `raw`, equality and a hash, and copies and pickles (#2561).
