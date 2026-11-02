---
section: Changed
---
- **Breaking (Rust API):** a colour object with storage (`storage_path` in
  Python) keeps a written default, so a zero default written over the network
  keeps the last value at the next start, and a durable object's idle save
  thread ends until its next save (#1956).
