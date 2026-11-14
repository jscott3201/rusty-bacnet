---
section: Changed
---
- **Breaking (Rust API):** `bacnet_tool::safety` writes go by address:
  `WriteSpec`, `Preview` and ledger entries carry it, so a write is read,
  gated, sent and reverted at one device; `turn` takes the address, `reach`
  replaces `find`. Ledgers without addresses still load (#2560).
