---
section: Changed
---
- **Breaking (Rust API):** `DataLink` is `#[non_exhaustive]`, so a `match` on it needs a wildcard arm (#2560).
