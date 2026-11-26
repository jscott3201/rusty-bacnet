---
section: Migration notes
---
- **Server and client refusals (Rust API, #1833):** the server's and client's
  own refusals are `Error::State(Lifecycle::.., _)`, `Error::InvalidConfig(_)`,
  `Error::InvalidArgument(_)`, `Error::Unsupported(_)` or
  `Error::LimitReached(_)` with the old text and no prefix, and their
  `OutOfRange` text is `Error::Range`; a request the client's stop cut off is
  `Error::RequestCutOff { sent }`. Match those variants.
