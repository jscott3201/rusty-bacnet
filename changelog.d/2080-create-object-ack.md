---
section: Changed
---
- **Breaking (Rust API, Python API):** `create_object` returns the created
  object's identifier, decoded by the new `CreateObjectAck`, in Rust and
  Python, and the CLI's `create-object` prints it instead of the
  acknowledgment's hex (#2080).
