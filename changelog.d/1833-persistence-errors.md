---
section: Changed
---
- **Breaking (Rust API):** persisted state that can't be loaded or saved is
  the new `Error::Persistence(PersistenceError)`, in every build, instead of
  `Error::Encoding`, `OutOfRange` or `InvalidConfig`; the codecs'
  `SnapshotError` is now this opaque `bacnet_types::error::PersistenceError`
  (#1833).
