---
section: Added
---
- `bacnet_ops::Client::bip()`, `bip6()`, `sc(..)` (with `sc-tls`) and
  `mstp(..)` (with the new `serial` feature) connect without naming a
  transport type, and `Client::over` wraps a low-level client (#2560).
