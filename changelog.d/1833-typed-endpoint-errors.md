---
section: Changed
---
- **Breaking (Rust API):** `bacnet-network`, `bacnet-endpoint-core` and
  `bacnet-endpoint` report lifecycle, configuration, argument, platform and
  limit refusals as `Error::State`, `InvalidConfig`, `InvalidArgument`,
  `Unsupported` and `LimitReached` instead of `Error::Encoding` text (#1833).
