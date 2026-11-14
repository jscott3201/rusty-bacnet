---
section: Changed
---
- **Breaking (Rust API):** `AsBip` is implemented for every `TransportPort` through `bip_transport`; a wrapper forwards that method instead of implementing `AsBip` (#2560).
