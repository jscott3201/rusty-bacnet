---
section: Migration notes
---
- **SC disconnect causes (Rust and Python API, #2438):** `PeerClose`
  (`peer_close`) now means a Disconnect-Request only. A WebSocket the hub
  closes or drops is `WebSocketClosed { close_status }` (`websocket_closed`),
  or `WebSocket(Closed)` or `WebSocket(StreamEnded)` (`websocket`). Match
  `TcpRefused` beside `TcpDial`, and `HubSubprotocolMissing` and
  `DirectSubprotocolMissing` beside the subprotocol kinds.
