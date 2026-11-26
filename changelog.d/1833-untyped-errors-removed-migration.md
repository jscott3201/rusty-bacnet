---
section: Migration notes
---
- **Typed errors (Rust API, #1833):** with `Error::Encoding` and
  `Error::OutOfRange` gone, an arm on either no longer compiles. Match the
  kind, as the other #1833 notes here describe per crate: `State`,
  `InvalidConfig`, `InvalidArgument`, `Unsupported`, `LimitReached`,
  `RequestCutOff`, `SendTooLong`, `Transport` or `Persistence`, and the
  codecs' `Encode`, `Decoding` and `Range`. To build one, use
  `Error::invalid_config(label)` and its siblings, or `Refusal::text`.
