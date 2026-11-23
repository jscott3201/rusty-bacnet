---
section: Changed
---
- **Breaking (Rust API):** `bacnet_endpoint::identity::client_config` and `apply_to_client_config`
  also give the client the identity's segmentation and segment timeout, as an endpoint session's
  client has them (#2617).
