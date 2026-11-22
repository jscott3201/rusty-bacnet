---
section: Migration notes
---
- **Tags snapshot (Rust API, #2395):** `TagsSnapshot` gains a `zone`
  field (`WrittenZone`); set it to `WrittenZone::default()` in a struct
  literal. A Device file holding a written zone half is `RBNTAG03`, which
  older builds refuse; other files keep their octets. `ClockReader` gains
  two provided methods.
