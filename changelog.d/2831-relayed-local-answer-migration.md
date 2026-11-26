---
section: Migration notes
---
- **Relayed local answers (wire, Rust API, #2831):** where a router relays a
  local device's answers back onto the same network, requests sent straight
  to that device time out unless it also answers from its own address, and
  `off_path_answer_drops` rises. `CanonicalPeer::routed_alias` is gone;
  answers match by `CanonicalPeer::of_answer`.
