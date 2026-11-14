---
section: Migration notes
---
- **bacnet-tool writes by address (Rust API, #2560):** give each
  `WriteSpec` the device's `address` beside `device`; read
  `Preview::address`; call `safety::turn(client, rate, &address, instance)`
  (fails with `safety::Unsent`) and `safety::reach` for `find`; pass the
  address and the client's own network number to `plan::undo_preview`.
  Ledger entries gain an optional `address`. Every `Caller` implements
  `may_look`; `revert_refusal` is optional.
