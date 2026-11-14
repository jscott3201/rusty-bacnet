---
section: Changed
---
- **Breaking (Rust API):** `bacnet-ops` watches points by address:
  `Client::watch` streams `Update`s for points from `Device::point`, a
  device that moves ends its points with `Why::Moved`, and `CovSupport`
  says what is known of a device's COV (#2560).
