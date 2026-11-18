---
section: Changed
---
- **Breaking (Rust API):** An endpoint session's stop settles its objects'
  saves through its server's stop, as a standalone server's does: a
  database guard the application holds no longer holds the stop up, and the
  objects settle once it is let go (#2617).
