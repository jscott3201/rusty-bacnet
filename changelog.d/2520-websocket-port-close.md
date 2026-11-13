---
section: Added
---
- **Rust API:** `WebSocketPort::close` and `WebSocketPort::has_close_frame`
  let a port send the WebSocket close frames the SC transport now sends; a
  custom port that keeps the defaults sends none (#2520).
