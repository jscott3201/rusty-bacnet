---
section: Changed
---
- **Breaking (Rust API):** a Load Control's application reports what its
  loads shed through a `LoadShedder` installed with `set_shedder`, which
  replaces `set_actual_shed_level` (#2464).
