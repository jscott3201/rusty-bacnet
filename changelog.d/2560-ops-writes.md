---
section: Added
---
- `bacnet-ops` writes: `Device::write`, `relinquish`, `priority_array` and
  `preview_write`, with a checked `Priority` and a `WritePolicy` that
  refuses priorities 1, 2, 5 and 6 unless the caller allows them, sending
  nothing (#2560).
