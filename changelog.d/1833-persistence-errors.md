---
section: Changed
---
- **Breaking (Rust API):** persisted state that can't be loaded or saved is
  the new `Error::Persistence(SnapshotError)`, in every build, instead of
  `Error::Encoding`, `OutOfRange` or `InvalidConfig`; `SnapshotError` moved
  to `bacnet_types::error` and gained a backend-failure kind (#1833).
