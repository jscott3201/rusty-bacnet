---
section: Changed
---
- **Breaking (Rust API):** `bacnet-transport` reports lifecycle, configuration,
  argument, platform and limit refusals as the new `Error::State`,
  `InvalidConfig`, `InvalidArgument`, `Unsupported` and `LimitReached`
  variants instead of `Error::Encoding` or `Error::OutOfRange` text (#1833).
