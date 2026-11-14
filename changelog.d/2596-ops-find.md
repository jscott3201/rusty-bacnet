---
section: Added
---
- **Rust API:** `bacnet_ops::Client::find` sends one Who-Has (`Find`: by
  name or identifier, scope, range, wait) under the rate cap and returns the
  objects that answered, a local broadcast unless it names another `Scope`;
  `raw::who_has` and `raw::who_has_replies` send one with no cap (#2596).
