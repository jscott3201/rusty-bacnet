---
section: Migration notes
---
- **Device storage (Rust API, #2730):** `ServerConfig` gains
  `device_storage_path` (`None` for none); exhaustive struct literals set it.
  A start refuses one left set, since only the builders build the Device it
  is for.
