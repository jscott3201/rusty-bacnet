---
section: Migration notes
---
- **Colour output storage (Rust API, #1956):** `ColorOutputSnapshot` gains
  `default`, and `present_value` becomes an `Option`; a custom
  `ColorOutputPersistence` stores both. Once a peer's default is saved,
  `set_default_color` and `set_default_color_temperature` keep it: call
  `clear_written_default` first (Python: `clear_written_default=True`) to
  apply yours. Old files still load.
