---
section: Migration notes
---
- **bacnet-device-core (Rust API, #2186):** `discovery::who_has` takes the
  `DeviceIdentity` first, as `who_is` does, in place of a Device identifier:
  `who_has(&identity, &db, request)`.
