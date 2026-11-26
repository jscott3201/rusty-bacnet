---
section: Changed
---
- **Breaking (Rust API):** a watch `Point` names its device by a
  `watch::Binding` (an address, or an instance the hub follows), `Update`
  gains `at`, `instance` and `moved`, and making a handle for an instance no
  longer moves the handles already following it (#2795).
