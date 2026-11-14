---
section: Changed
---
- **Breaking (Rust API):** `bacnet_tool::safety` writes go by address:
  `WriteSpec`, `Preview` and ledger entries carry it; `turn` takes the
  address and the instance and refuses another device there (`Unsent`),
  and `reach` replaces `find`. Ledgers without addresses still load
  (#2560).
