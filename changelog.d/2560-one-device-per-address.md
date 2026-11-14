---
section: Changed
---
- **Breaking (Rust and Python API):** the discovery table holds one device
  per address: an I-Am, `add_device` or `upsert` naming another instance at
  a known address replaces its row, so `get_device` no longer finds the old
  instance, and instances aliased at one MAC no longer coexist (#2560).
