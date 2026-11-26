---
section: Migration notes
---
- **Network and endpoint refusals (Rust API, #1833):** the network layer's,
  router's and endpoint's own refusals are now `Error::State(Lifecycle::..,
  _)`, `Error::InvalidConfig(_)`, `Error::InvalidArgument(_)`,
  `Error::Unsupported(_)` or `Error::LimitReached(_)`, with the old text and
  no prefix; a session that isn't running says `NotRunning`, `Stopping` or
  `Closed`. Match those variants.
