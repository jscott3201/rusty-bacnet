---
section: Added
---
- `Device::read_many` in `bacnet-ops` reads many properties in the watch
  hub's batches and never fails as a whole: a refused property is an error
  in its own row, and a device that stops answering keeps what was read
  (#2560).
