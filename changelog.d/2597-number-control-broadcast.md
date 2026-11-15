---
section: Changed
---
- **Breaking (Rust API):** `bacnet_network::network_number::NumberControl::WhatIs`
  carries `broadcast`; `ANSWER_DELAY_MIN` and `ANSWER_DELAY_MAX` give the
  bounds of a node's wait before its Network-Number-Is answer (#2597).
