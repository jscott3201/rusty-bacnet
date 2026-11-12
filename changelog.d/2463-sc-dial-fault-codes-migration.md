---
section: Migration notes
---
- **SC dial error kinds (Rust and Python API, #2463):** match
  `AddressUnreachable` beside `TcpDial`, and `WebSocketHeader`,
  `UpgradeResponseSyntax` and `UpgradeResponseValue` beside
  `WebSocketHandshake`. Dials that reported `tcp-dial` or
  `websocket-handshake` for those faults now report `address-unreachable`,
  `websocket-header`, `upgrade-response-syntax` or `upgrade-response-value`
  (Python's `cause_detail`).
