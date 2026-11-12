---
section: Migration notes
---
- **SC dial error kinds (Rust and Python API, #2463):** match
  `AddressUnreachable` beside `TcpDial`, and `WebSocketHeader` and
  `UpgradeResponseSyntax` beside `WebSocketHandshake`. Dials that reported
  `tcp-dial` or `websocket-handshake` for those faults now report
  `address-unreachable`, `websocket-header` or `upgrade-response-syntax`
  (Python's `cause_detail`).
