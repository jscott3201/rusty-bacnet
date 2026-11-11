---
section: Migration notes
---
- **Local writes (Rust API, #2446):** import `LocalCommandSource` from
  `bacnet_device_core::write::local` (add `bacnet-device-core` to your
  dependencies); `bacnet_server::LocalCommandSource` and the
  `bacnet_server::command_source` module are gone. `write_local` and
  `write_local_encoded` take it as before.
