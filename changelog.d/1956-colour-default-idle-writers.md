---
section: Changed
---
- **Breaking (Rust API):** a colour object with storage keeps a written
  default, which wins over `set_default_color*` until `clear_written_default`
  drops it, so a zero default written over the network keeps the last value
  at the next start (#1956).
