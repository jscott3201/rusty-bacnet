---
section: Migration notes
---
- **Object and device-core refusals (Rust API, #1833):** a refused identity,
  builder, budget or COV policy is now `Error::InvalidConfig(_)`, a refused
  argument `Error::InvalidArgument(_)`, an exhausted counter or held port
  `Error::LimitReached(_)`, a number outside its bounds `Error::Range`, and
  saved state that can't be restored `Error::Persistence(_)`, with the old
  text. Match those variants.
