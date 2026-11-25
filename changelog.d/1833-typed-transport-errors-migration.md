---
section: Migration notes
---
- **Transport refusals (Rust API, #1833):** a transport's own refusals are now
  `Error::State(Lifecycle::.., _)`, `Error::InvalidConfig(_)`,
  `Error::InvalidArgument(_)`, `Error::Unsupported(_)` or
  `Error::LimitReached(_)`, with the old text and no prefix. Match those
  variants: an `Error::Encoding(_)` or `Error::OutOfRange(_)` arm still
  compiles, but silently stops matching them.
