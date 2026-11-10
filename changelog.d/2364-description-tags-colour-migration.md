---
section: Migration notes
---
- **Snapshot formats (Rust API, #2364):** `TagsSnapshot` and
  `ColorOutputSnapshot` gain a `description` field, and
  `ColorOutputSnapshot` is no longer `Copy`: clone it. A file holding a
  Description is `RBNTAG02` or `RBNCLR03`, which older builds refuse;
  files without one keep their octets. Once one is saved, `set_description`
  on a Device or lighting output gives way until
  `clear_written_description`.
