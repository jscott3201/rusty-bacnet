---
section: Added
---
- `bacnet-ops`: `Device::point_records` reads the inventory's records of the objects a caller
  names, in the inventory's batches and in the order named, and `Device::point_batch` says how
  many one request reads, for front ends that page a device's objects (#2785).
