---
section: Changed
---
- **Breaking (Rust API):** `LoadControlSnapshot` gains `shed_levels`, saved
  as `RBNLCT02`, and is `#[non_exhaustive]`; the new
  `LoadControlObject::clear_written_shed_levels` drops the saved levels so
  `set_shed_levels` applies whole (#2502).
