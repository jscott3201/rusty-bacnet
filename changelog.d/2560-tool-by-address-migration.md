---
section: Migration notes
---
- **bacnet-tool writes by address (Rust API, #2560):** give each
  `WriteSpec` the device's `address` (from its handle or discovery-table
  row) beside `device`; read `Preview::address`; call
  `safety::turn(client, rate, &address)` and `safety::reach(client,
  &address, instance)` for `find`; pass the entries' address to
  `plan::undo_preview`. Ledger entries gain an optional `address`.
