---
section: Removed
---
- **Breaking (Rust API):** `Error::Encoding(String)` and
  `Error::OutOfRange(String)` are gone: every error the stack returns has a
  typed variant, and no variant is text alone (#1833).
