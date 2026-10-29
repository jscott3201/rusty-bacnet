---
section: Changed
---
- **Breaking (Rust API, Python API):** A server or endpoint refuses a
  configured DCC or ReinitializeDevice password that is empty or over 20
  characters: at start in Rust, with ValueError at construction in Python. An
  empty password from a peer counts as none (#2060, #2094).
