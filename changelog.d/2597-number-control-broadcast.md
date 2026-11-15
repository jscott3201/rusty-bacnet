---
section: Changed
---
- **Breaking (Rust API):** `bacnet_network::network_number::NumberControl::WhatIs`
  carries `broadcast`, and the new `DelayedAnswer` holds a node's
  Network-Number-Is answer for its wait (#2597).
