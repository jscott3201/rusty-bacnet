---
section: Added
---
- **Rust API:** `bacnet_ops::Client::find` sends one Who-Has (`Find`) under
  the rate cap, local unless it names another `Scope`, and returns the
  objects that answered from that scope; `raw::who_has` and
  `raw::who_has_replies` send one with no cap (#2596).
