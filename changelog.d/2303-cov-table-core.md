---
section: Changed
---
- **Breaking (Rust API):** The COV subscription table, SubscribeCOV and
  SubscribeCOVProperty move into `bacnet-device-core`'s `cov` module, on
  `bacnet-time`, so firmware can keep subscriptions; `bacnet-server` runs its
  table on it (#2303).
