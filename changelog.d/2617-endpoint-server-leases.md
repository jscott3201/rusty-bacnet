---
section: Changed
---
- **Breaking (Rust and Python API):** An endpoint's client and server share
  its 256 invoke IDs by reserves: with both roles the client always finds
  128 and may hold 192 (256 before, with the server idle), the server 64
  and up to 128 (a fixed 64 before). One role alone may take all 256
  (#2617).
