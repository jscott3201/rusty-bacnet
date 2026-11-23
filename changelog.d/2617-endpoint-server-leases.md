---
section: Changed
---
- **Rust API:** An endpoint session's client and server share its 256
  invoke IDs by reserves: with both roles the client always finds 128 and
  may hold 192, the server always finds 64 and may hold 128, in place of the
  server's cap of 64. A one-role session's role may take all 256 (#2617).
