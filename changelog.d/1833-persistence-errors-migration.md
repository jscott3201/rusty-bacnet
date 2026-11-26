---
section: Migration notes
---
- **Persisted state (Rust API, #1833):** match `Error::Persistence(e)` where a
  snapshot, saved state or a storage backend failed, and read `e.kind()` and
  `e.is_damage()`. `bacnet_objects::snapshot::SnapshotError` is
  `bacnet_types::error::PersistenceError`, opaque: build one with
  `PersistenceError::new(kind, detail)` (a backend's own failure is kind
  `BackendFailed`); `SnapshotErrorKind` is `PersistenceErrorKind`, its
  `OutOfRange` now `RefusedOnEncode`.
