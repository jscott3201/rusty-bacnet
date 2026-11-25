---
section: Changed
---
- **Breaking (Rust API):** `bacnet-server` and `bacnet-client` report lifecycle,
  configuration, argument, platform and limit refusals as `Error::State`,
  `InvalidConfig`, `InvalidArgument`, `Unsupported` and `LimitReached` instead
  of `Error::Encoding` text; wire replies are unchanged (#1833).
