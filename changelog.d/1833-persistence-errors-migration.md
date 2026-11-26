---
section: Migration notes
---
- **Persisted state (Rust API, #1833):** match `Error::Persistence(e)` where a
  snapshot, a saved configuration or a storage backend failed, and read
  `e.kind()` and `e.is_damage()` instead of the text. Import `SnapshotError`
  from `bacnet_types::error` (`bacnet_objects::snapshot` re-exports it). A
  storage backend of your own reports its failure with
  `SnapshotError::backend_failed`. The Tags file backend, which names its
  path, reports a file it can't load as `Error::InvalidConfig`.
