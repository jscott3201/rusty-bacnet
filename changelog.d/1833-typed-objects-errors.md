---
section: Changed
---
- **Breaking (Rust API):** `bacnet-objects` and `bacnet-device-core` report
  their refusals as `Error::InvalidConfig`, `InvalidArgument`, `Unsupported`,
  `LimitReached` or `Range` instead of `Error::Encoding` or
  `Error::OutOfRange` text; wire replies are unchanged (#1833).
