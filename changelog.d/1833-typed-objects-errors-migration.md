---
section: Migration notes
---
- **Object and device-core refusals (Rust API, #1833):** a refused identity,
  builder, budget, COV policy or saved configuration is now
  `Error::InvalidConfig(_)`, a refused argument `Error::InvalidArgument(_)`,
  an exhausted counter or held port `Error::LimitReached(_)`, and an
  `OutOfRange` text `Error::Range`, with the old text. Match those variants:
  an `Error::Encoding(_)` or `Error::OutOfRange(_)` arm still compiles, but
  silently stops matching them.
