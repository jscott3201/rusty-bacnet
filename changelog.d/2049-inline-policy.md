---
section: Added
---
- **Rust API:** A `ServerPolicy` that never blocks can return `true` from
  `decides_inline()` (or use `FnPolicy::inline()`) to be called on the
  request task, skipping the blocking-pool trip each decision otherwise
  takes; Python's `mutation_policy` stays on the pool (#2049).
