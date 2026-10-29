---
section: Migration notes
---
- **Registered B/IPv6 port (Rust API, #2057):** build the `Bip6Transport` with
  `Some(instance)` of the Device the server serves; a different instance or
  `None` (a random VMAC) now fails at start.
