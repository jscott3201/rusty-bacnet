---
section: Migration notes
---
- **Colour output storage (Rust API, #1956):** `ColorOutputSnapshot` gains
  `default`, and `present_value` becomes an `Option` (`None` before anything
  drove the output). A custom `ColorOutputPersistence` stores both fields.
  `FileColorOutputPersistence` writes a new version of its file and still
  loads the old one.
