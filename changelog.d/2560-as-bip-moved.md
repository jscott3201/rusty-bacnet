---
section: Changed
---
- **Breaking (Rust API):** the `AsBip` trait is gone; `as_bip` is a method on `TransportPort`, and wrappers forward `data_link` and `bip_transport` (#2560).
