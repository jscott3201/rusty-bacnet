---
section: Added
---
- **Rust API:** `WebSocketPort::close` sends the WebSocket close frames the SC
  transport now sends, and `WebSocketPort::awaits_disconnect_ack` turns on its
  wait for a Disconnect-ACK; a custom port that keeps both defaults behaves as
  before (#2520).
