---
section: Migration notes
---
- **Transport abort (Rust API, #2617):** `fn abort(&mut self)` becomes
  `fn abort(&self)`: do at once, without waiting or a goodbye, what the
  stop does, fail every later send with `port::stopped()`, and leave the
  aborted tasks for a later stop to join.
