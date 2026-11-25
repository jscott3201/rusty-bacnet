---
section: Added
---
- `bacnet-ops`: `Device::point_records` reads the inventory's records of the objects named, in
  its batches and order; `Device::point_batch` says how many one request reads, and
  `Client::rpm_limit` the smaller request size a device was found to answer (#2785).
