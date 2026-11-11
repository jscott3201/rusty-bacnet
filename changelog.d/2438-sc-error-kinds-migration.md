---
section: Migration notes
---
- **SC dial error kinds (Rust and Python API, #2438):** match `TcpRefused`
  beside `TcpDial`. A real dial whose upgrade answer lacked or mis-stated the
  subprotocol reported `WebSocketHandshake` (`websocket-handshake`); it now
  reports `HubSubprotocolMissing` or `HubSubprotocol`, and their `Direct`
  counterparts on a direct dial.
