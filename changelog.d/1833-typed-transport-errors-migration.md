---
section: Migration notes
---
- **Transport errors (Rust API, #1833):** match `Error::State(Lifecycle::..,
  _)`, `Error::InvalidConfig(_)`, `Error::InvalidArgument(_)`,
  `Error::Unsupported(_)` or `Error::LimitReached(_)` where code matched a
  transport's `Error::Encoding` or `Error::OutOfRange`; `to_string()` gives
  the same text as before, without its prefix.
