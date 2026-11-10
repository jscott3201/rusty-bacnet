---
section: Migration notes
---
- **Snapshot formats (Rust API, #2364):** `TagsSnapshot` and
  `ColorOutputSnapshot` gain `description`, and the second is no longer
  `Copy`: clone it. A `TagsPersistence` or `ColorOutputPersistence` backend
  of your own must store and return it. Files holding one are `RBNTAG02` or
  `RBNCLR03`, which older builds refuse. With storage, a Device, lighting
  output or colour object now refuses a written Description past 16 KiB,
  and a saved one wins over `set_description` until
  `clear_written_description`.
