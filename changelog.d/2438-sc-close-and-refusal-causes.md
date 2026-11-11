---
section: Changed
---
- **Breaking (Rust and Python API):** a WebSocket the SC hub closes or drops
  is no longer a peer close: it keeps the close status or reads as an
  abnormal end, and a refused dial and a missing or wrong subprotocol have
  their own kinds, each with its own error code (#2438).
